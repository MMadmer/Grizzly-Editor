// Command github_mock is a local stand-in for the GitHub and VirusTotal endpoints the editor talks to.
//
// Covers, for Publish Release: the REST API of the mod's own repository (--repo:
// repository, releases, release assets, uploads, tag refs), the anonymous web
// download path (releases/latest/download -> CDN redirect) and the OAuth device
// flow. For Submit to Mod Browser: GET /user and the catalog repository
// (--catalog-repo) - the repository, its files (GET contents), its issues (list,
// open, read, edit) with their comments - and the catalog's public files on the
// web (/catalog/public/..). For the package checks: VirusTotal's v3 file API
// under /vt/api/v3 (files, files/upload_url, analyses) with the author's own key.
// A set of fault switches comes with them (see --help). All state lives in
// memory. Self-test: ../test_github_mock.py.
//
// The first stdout line is {"port": N}. Nothing else is printed unless a request
// handler fails, so a harness may leave the pipes unread.
//
// Harness routes take no token and are never logged:
//
//	GET  /__state       the releases, tags, upload count and request log
//	POST /__shutdown
//	GET  /__catalog     {"repositories": {"<catalog>": {"branches": {"<name>":
//	                    {"sha", "message", "files": {path: text},
//	                    "files_base64": {path: base64}}}}}, "issues": [the issue
//	                    JSON plus "comment_list": [the comment JSON]]}, oldest first
//	POST /__catalog/file?path=P[&branch=B]
//	                    the request body becomes P in one commit on B (main): the
//	                    catalog committing an accepted file or publishing its index
//	POST /__catalog/issue?creator=LOGIN&title=T[&pull=1]
//	                    an issue opened by anyone, the request body (UTF-8, may be
//	                    empty) its text; pull=1 makes it an entry standing for a
//	                    pull request
//	POST /__catalog/comment?number=N[&author=LOGIN][&count=K]
//	                    K comments (1..1000, default 1) on N, the request body
//	                    (UTF-8) their text; answers the list of them
//	POST /__catalog/close?number=N[&reason=completed|not_planned][&by=LOGIN]
//	                    closes N, which has to be open (422), as completed unless
//	                    reason= says otherwise
//	GET  /__vt, POST /__vt/seed
//	                    VirusTotal's files and analyses; verdicts for known files
//	POST /__vt/queue?sha=S&lookups=N
//	                    S is being analysed: its report stays out for N lookups (a
//	                    file VirusTotal did not know gets --vt-upload-results)
//	POST /__vt/forget?sha=S
//	                    VirusTotal no longer knows S, as if it had never seen it
//
// author= and by= default to github-actions[bot], the account a workflow writes
// as: that is how a test plays the catalog's workflow - and a stranger forging it.
//
// Behaviour worth knowing before writing a client against it:
//
//   - Every response closes its connection. The request body is always consumed
//     first, so an error answer is never lost to a connection reset.
//   - API routes (/user, /repos, /upload) demand a User-Agent header, as GitHub
//     does, then the bearer token, then the repository name.
//   - "authorized" in the request log means the request carried an Authorization
//     header, whatever its value. Web, CDN, device-flow, /catalog/public and /vt/
//     requests must show false: a true there is a client leaking its token.
//     "vt_key" means it carried an x-apikey header, whatever its value: only /vt/
//     requests may show true.
//   - Publishing a release creates its git tag, deleting the release leaves the tag
//     behind, and deleting the tag of a release that is still published turns that
//     release into a draft - all three as on GitHub, which is why a client that
//     removes a release removes the release first and the tag second.
//   - --repo keeps its releases-only model. The catalog has files and issues and
//     nothing else - no forks, git refs, content writes or pull request routes: a
//     submission is an issue, and the catalog's workflow judges it, comments,
//     commits the file itself and closes the issue.
//   - Issues and the entries standing for pull requests share one numbering from
//     1, as on GitHub, and GET issues lists both, newest first: state=open by
//     default, creator= matched case-insensitively, per_page / page as every list
//     here. A pull entry carries "pull_request", a plain issue no such key. An
//     issue without text has the body null.
//   - POST issues and POST issues/{n}/comments write as --login; a comment goes on
//     any issue, open or closed. PATCH issues/{n} needs --login to be the issue's
//     author or the catalog's owner (403 "Must have admin rights to Repository."
//     otherwise), and an author who is not the owner cannot reopen what somebody
//     else closed. Closing sets closed_at, closed_by and state_reason (completed
//     unless given); reopening clears the first two and says reopened. The
//     state_reason counts only when the state changes, and then it has to fit the
//     change. A refused PATCH changes nothing; an accepted one, a comment and a
//     closing move updated_at.
//   - VirusTotal: an Authorization header on a /vt/ request answers 400 before the
//     key is looked at, a wrong or missing x-apikey 401. Only requests with the
//     right key count toward --vt-rate, and a 429 does not count.
//   - The VirusTotal size limits apply to the request body, the multipart envelope
//     included: a file of exactly --vt-max-upload bytes does not fit POST /files.
//   - An upload URL is spent by the first POST that reaches it with the right key,
//     whatever that POST ends with.
//   - Every upload queues the file's report for --vt-queue lookups; GET /files/{sha}
//     and GET /analyses/{id} of a queued analysis both count. A known file uploaded
//     again keeps its verdicts, a new one gets --vt-upload-results, and a seeded
//     one reports size 0 until it is uploaded. While queued, GET /files answers a
//     report without results - or, with --vt-hide-new, 404 for a file whose FIRST
//     analysis is not done yet, which is what VirusTotal does.
package main

import (
	"bytes"
	"fmt"
	"io"
	"io/fs"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
)

func main() {
	cfg := parseArgs(os.Args[1:])
	m := newMock(cfg)

	if cfg.seed != "" {
		value, err := readJSONFile(cfg.seed)
		if err == nil {
			err = m.seedReleases(value)
		}
		if err != nil {
			die("--seed: %v", err)
		}
	}
	files := map[string][]byte{}
	if cfg.catalogSeed != "" {
		var err error
		if files, err = readTree(cfg.catalogSeed); err != nil {
			die("--catalog-seed: %v", err)
		}
	}
	m.seedCatalog(files)
	if cfg.vtSeed != "" {
		value, err := readJSONFile(cfg.vtSeed)
		var seed object
		if err == nil {
			seed, err = readVtSeed(value)
		}
		if err != nil {
			die("--vt-seed: %v", err)
		}
		m.vtApplySeed(seed)
	}
	if cfg.log != "" {
		file, err := os.OpenFile(cfg.log, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644)
		if err != nil {
			die("--log: %v", err)
		}
		m.logFile = file
	}

	listener, err := net.Listen("tcp4", net.JoinHostPort("127.0.0.1", strconv.Itoa(cfg.port)))
	if err != nil {
		die("%v", err)
	}
	m.port = listener.Addr().(*net.TCPAddr).Port
	fmt.Printf("{\"port\": %d}\n", m.port)

	server := &http.Server{Handler: m, ReadHeaderTimeout: socketTimeout, ErrorLog: log.New(io.Discard, "", 0)}
	go server.Serve(listener)

	interrupt := make(chan os.Signal, 1)
	signal.Notify(interrupt, os.Interrupt)
	select {
	case <-m.stop:
	case <-interrupt:
	}
	listener.Close()
}

func die(format string, args ...any) {
	fmt.Fprintf(os.Stderr, format+"\n", args...)
	os.Exit(1)
}

func readJSONFile(path string) (any, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	value, valid := parseJSON(bytes.TrimPrefix(raw, []byte("\xef\xbb\xbf")))
	if !valid {
		return nil, fmt.Errorf("%s is not valid JSON", path)
	}
	return value, nil
}

// readTree loads every file below folder, keyed by its relative path with '/' separators.
func readTree(folder string) (map[string][]byte, error) {
	files := map[string][]byte{}
	err := filepath.WalkDir(folder, func(path string, _ fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		// Stat, not the entry: a link to a folder is listed as a file and is not walked.
		if info, statErr := os.Stat(path); statErr == nil && info.IsDir() {
			return nil
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		relative, err := filepath.Rel(folder, path)
		if err != nil {
			return err
		}
		files[filepath.ToSlash(relative)] = data
		return nil
	})
	return files, err
}
