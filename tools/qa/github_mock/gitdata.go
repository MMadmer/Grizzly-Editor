package main

// The catalog's Git Data API, its file history and its workflow runs, for the maintainer's own client (Catalog Desk,
// a program of its own beside the editor): the catalog read as a tree of blobs at a commit, a commit of several files
// made over the head of a branch and moved there by fast-forward only, the commits that changed a path, and
// workflow runs - a push that touches mods/, catalog.ltx or revoked.ltx starts publish.yml, as the catalog's
// push filter does; a dispatch starts the workflow it names.
//
// Harness routes:
//
//	GET  /__catalog/log[?branch=B]   the branch's commits, newest first: sha, message, author, date
//	GET  /__catalog/runs             every workflow run, oldest first: workflow, event
//	POST /__catalog/race?path=P      the next ref update of a client is beaten by the catalog's workflow,
//	                                 which commits the request body as P first: the client's commit is no
//	                                 fast-forward any more (422), as on GitHub
//	POST /__release/age?id=N&minutes=M
//	                                 release N and its assets were made M minutes earlier than they were: an
//	                                 asset uploaded now is then one uploaded after the release went out

import (
	"bytes"
	"cmp"
	"crypto/sha1"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"maps"
	"slices"
	"strings"
	"time"
)

type catalogRun struct {
	id                       int
	workflow, event, created string
}

// gitState is the Git Data API's own store: blobs a client posted and the trees it built over them.
type gitState struct {
	blobs     map[string][]byte
	trees     map[string]map[string][]byte
	runs      []*catalogRun
	nextRunID int
	racePath  string
	raceData  []byte
	armed     bool
}

func (m *mock) git() *gitState {
	if m.gitData == nil {
		m.gitData = &gitState{blobs: map[string][]byte{}, trees: map[string]map[string][]byte{}, nextRunID: 9000000001}
	}
	return m.gitData
}

func treeSHA(files map[string][]byte) string {
	digest := sha1.New()
	for _, path := range slices.Sorted(maps.Keys(files)) {
		fmt.Fprintf(digest, "%s\x00%s\n", path, gitBlobSHA(files[path]))
	}
	return hex.EncodeToString(digest.Sum(nil))
}

// treeOf registers a commit's files as a tree and names it.
func (m *mock) treeOf(files map[string][]byte) string {
	sha := treeSHA(files)
	m.git().trees[sha] = files
	return sha
}

// blob is a posted blob or a file of any commit: blob ids name their bytes.
func (m *mock) blob(sha string) ([]byte, bool) {
	if data, present := m.git().blobs[sha]; present {
		return data, true
	}
	for _, c := range m.commits {
		for _, data := range c.files {
			if gitBlobSHA(data) == sha {
				return data, true
			}
		}
	}
	return nil, false
}

func (m *mock) gitCommitJSON(c *commit) object {
	parents := listOf(c.parents, func(sha string) any { return object{{"sha", sha}} })
	author := c.author
	if author == nil {
		author = object{{"name", "catalog-bot"}, {"email", "catalog-bot@users.noreply.github.com"}, {"date", c.date}}
	}
	return object{
		{"sha", c.sha},
		{"tree", object{{"sha", m.treeOf(c.files)}}},
		{"parents", parents},
		{"message", c.message},
		{"author", author},
		{"committer", author},
	}
}

// routeCatalogGit answers the catalog's git/, commits and actions/ routes; handled is false for every other route.
func (x *exchange) routeCatalogGit(rest []string, raw []byte) (result *answer, handled bool, err error) {
	m, method := x.mock, x.r.Method
	answer := func(a *answer, e error) (*answer, bool, error) { return a, true, e }
	switch {
	case method == "GET" && len(rest) == 4 && match(rest[:3], "git", "ref", "heads"):
		sha, present := m.catalog.branches[rest[3]]
		if !present {
			return answer(nil, notFound())
		}
		return answer(ok(object{{"ref", "refs/heads/" + rest[3]}, {"object", object{{"sha", sha}, {"type", "commit"}}}}, nil))
	case method == "GET" && match(rest, "git", "commits", "*"):
		c, present := m.commits[strings.ToLower(rest[2])]
		if !present {
			return answer(nil, notFound())
		}
		return answer(ok(m.gitCommitJSON(c), nil))
	case method == "GET" && match(rest, "git", "trees", "*"):
		files, present := m.git().trees[strings.ToLower(rest[2])]
		if !present {
			return answer(nil, notFound())
		}
		entries := []any{}
		for _, path := range slices.Sorted(maps.Keys(files)) {
			entries = append(entries, object{{"path", path}, {"mode", "100644"}, {"type", "blob"},
				{"sha", gitBlobSHA(files[path])}, {"size", len(files[path])}})
		}
		return answer(ok(object{{"sha", rest[2]}, {"tree", entries}, {"truncated", false}}, nil))
	case method == "GET" && match(rest, "git", "blobs", "*"):
		data, present := m.blob(strings.ToLower(rest[2]))
		if !present {
			return answer(nil, notFound())
		}
		if strings.Contains(x.r.Header.Get("Accept"), "raw") {
			return answer(text(data, "application/vnd.github.raw"), nil)
		}
		return answer(ok(object{{"sha", rest[2]}, {"size", len(data)}, {"encoding", "base64"}, {"content", wrappedBase64(data)}}, nil))
	case method == "POST" && match(rest, "git", "blobs"):
		if !m.isMaintainer() {
			return answer(nil, noAdminRights())
		}
		body, e := jsonBody(raw)
		if e != nil {
			return answer(nil, e)
		}
		content, _ := body.text("content")
		encoding, _ := body.text("encoding")
		data := []byte(content)
		if encoding == "base64" {
			if data, e = base64.StdEncoding.DecodeString(content); e != nil {
				return answer(nil, validationFailed("Blob", "invalid", "content"))
			}
		} else if encoding != "" && encoding != "utf-8" {
			return answer(nil, validationFailed("Blob", "invalid", "encoding"))
		}
		sha := gitBlobSHA(data)
		m.git().blobs[sha] = data
		return answer(created(object{{"sha", sha}, {"url", m.catalogAPI() + "/git/blobs/" + sha}}, nil))
	case method == "POST" && match(rest, "git", "trees"):
		if !m.isMaintainer() {
			return answer(nil, noAdminRights())
		}
		body, e := jsonBody(raw)
		if e != nil {
			return answer(nil, e)
		}
		return answer(created(m.postTree(body)))
	case method == "POST" && match(rest, "git", "commits"):
		if !m.isMaintainer() {
			return answer(nil, noAdminRights())
		}
		body, e := jsonBody(raw)
		if e != nil {
			return answer(nil, e)
		}
		return answer(created(m.postCommit(body)))
	case method == "PATCH" && len(rest) == 4 && match(rest[:3], "git", "refs", "heads"):
		if !m.isMaintainer() {
			return answer(nil, noAdminRights())
		}
		body, e := jsonBody(raw)
		if e != nil {
			return answer(nil, e)
		}
		return answer(ok(m.moveBranch(rest[3], body)))
	case method == "GET" && match(rest, "commits"):
		return answer(ok(m.pathHistory(cmp.Or(x.query["sha"], defaultBranch), x.query["path"], x.query)))
	case method == "GET" && match(rest, "actions", "workflows", "*", "runs"):
		runs := []*catalogRun{}
		for _, run := range newestFirst(m.git().runs) {
			if run.workflow == rest[2] {
				runs = append(runs, run)
			}
		}
		page := listOf(pageOf(runs, x.query), func(run *catalogRun) any { return m.runJSON(run) })
		return answer(ok(object{{"total_count", len(runs)}, {"workflow_runs", page}}, nil))
	case method == "POST" && match(rest, "actions", "workflows", "*", "dispatches"):
		if !m.isMaintainer() {
			return answer(nil, noAdminRights())
		}
		body, e := jsonBody(raw)
		if e != nil {
			return answer(nil, e)
		}
		if ref, _ := body.text("ref"); m.catalog.branches[strings.TrimPrefix(ref, "refs/heads/")] == "" {
			return answer(nil, validationFailed("Workflow", "invalid", "ref"))
		}
		m.startRun(rest[2], "workflow_dispatch")
		return answer(noContent(nil))
	}
	return nil, false, nil
}

func (m *mock) postTree(body object) (any, error) {
	baseSHA, _ := body.text("base_tree")
	base, present := m.git().trees[strings.ToLower(baseSHA)]
	if baseSHA != "" && !present {
		return nil, validationFailed("Tree", "invalid", "base_tree")
	}
	files := maps.Clone(base)
	if files == nil {
		files = map[string][]byte{}
	}
	value, _ := body.get("tree")
	entries, isList := value.([]any)
	if !isList {
		return nil, validationFailed("Tree", "missing_field", "tree")
	}
	for _, item := range entries {
		entry, isObject := item.(object)
		path, hasPath := entry.text("path")
		if !isObject || !hasPath || path == "" {
			return nil, validationFailed("Tree", "invalid", "tree")
		}
		sha, _ := entry.get("sha")
		if sha == nil {
			// A null sha takes the path out of the base tree.
			delete(files, path)
			continue
		}
		data, known := m.blob(strings.ToLower(fmt.Sprint(sha)))
		if mode, _ := entry.text("mode"); !known || mode != "100644" {
			return nil, validationFailed("Tree", "invalid", "tree.sha")
		}
		files[path] = data
	}
	sha := m.treeOf(files)
	return object{{"sha", sha}, {"url", m.catalogAPI() + "/git/trees/" + sha}}, nil
}

func (m *mock) postCommit(body object) (any, error) {
	treeText, _ := body.text("tree")
	files, present := m.git().trees[strings.ToLower(treeText)]
	if !present {
		return nil, validationFailed("Commit", "invalid", "tree")
	}
	value, _ := body.get("parents")
	list, _ := value.([]any)
	parents := []string{}
	for _, parent := range list {
		sha := strings.ToLower(fmt.Sprint(parent))
		if _, known := m.commits[sha]; !known {
			return nil, validationFailed("Commit", "invalid", "parents")
		}
		parents = append(parents, sha)
	}
	message, _ := body.text("message")
	c := m.newCommit(maps.Clone(files), parents, message)
	if author, given := body.get("author"); given {
		c.author, _ = author.(object)
	}
	return m.gitCommitJSON(c), nil
}

// moveBranch: PATCH git/refs/heads/{branch}; without force only to a commit whose first parent is the head.
func (m *mock) moveBranch(branch string, body object) (any, error) {
	head, present := m.catalog.branches[branch]
	if !present {
		return nil, refusal(422, "Reference does not exist")
	}
	g := m.git()
	if g.armed {
		g.armed = false
		files := maps.Clone(m.commits[head].files)
		files[g.racePath] = g.raceData
		head = m.newCommit(files, []string{head}, "Publish the catalog").sha
		m.catalog.branches[branch] = head
	}
	sha, _ := body.text("sha")
	target, known := m.commits[strings.ToLower(sha)]
	if !known {
		return nil, refusal(422, "Object does not exist")
	}
	if force, _ := body.get("force"); !truthy(force) && (len(target.parents) == 0 || target.parents[0] != head) {
		return nil, refusal(422, "Update is not a fast forward")
	}
	m.catalog.branches[branch] = target.sha
	if branch == defaultBranch && touchesPublish(m.commits[head].files, target.files) {
		m.startRun("publish.yml", "push")
	}
	return object{{"ref", "refs/heads/" + branch}, {"object", object{{"sha", target.sha}, {"type", "commit"}}}}, nil
}

// touchesPublish: the catalog's publish.yml starts on a push that changes mods/, catalog.ltx or revoked.ltx.
func touchesPublish(before, after map[string][]byte) bool {
	for _, files := range []map[string][]byte{before, after} {
		for path := range files {
			if !bytes.Equal(before[path], after[path]) || (before[path] == nil) != (after[path] == nil) {
				if strings.HasPrefix(path, "mods/") || path == "catalog.ltx" || path == "revoked.ltx" {
					return true
				}
			}
		}
	}
	return false
}

// pathHistory: GET commits?sha=&path= - the first-parent line of a branch, newest first, the commits that changed path.
func (m *mock) pathHistory(ref, path string, query map[string]string) (any, error) {
	sha := m.catalog.branches[strings.TrimPrefix(ref, "refs/heads/")]
	if sha == "" {
		if _, known := m.commits[strings.ToLower(ref)]; !known {
			return nil, refusal(404, "No commit found for SHA: "+ref)
		}
		sha = strings.ToLower(ref)
	}
	found := []*commit{}
	for c := m.commits[sha]; c != nil; {
		var parent *commit
		if len(c.parents) > 0 {
			parent = m.commits[c.parents[0]]
		}
		var before []byte
		wasThere := false
		if parent != nil {
			before, wasThere = parent.files[path]
		}
		now, isThere := c.files[path]
		if path == "" || isThere != wasThere || !bytes.Equal(now, before) {
			found = append(found, c)
		}
		c = parent
	}
	return listOf(pageOf(found, query), func(c *commit) any {
		json := m.gitCommitJSON(c)
		return object{{"sha", c.sha}, {"commit", object{{"message", c.message}, {"author", json[4].value}, {"committer", json[5].value}}}}
	}), nil
}

func (m *mock) startRun(workflow, event string) {
	g := m.git()
	g.runs = append(g.runs, &catalogRun{g.nextRunID, workflow, event, timestamp()})
	g.nextRunID++
}

func (m *mock) runJSON(run *catalogRun) object {
	return object{
		{"id", run.id},
		{"name", strings.TrimSuffix(run.workflow, ".yml")},
		{"status", "completed"},
		{"conclusion", "success"},
		{"event", run.event},
		{"created_at", run.created},
		{"updated_at", run.created},
		{"head_sha", m.catalog.branches[defaultBranch]},
		{"html_url", fmt.Sprintf("%s/%s/actions/runs/%d", m.origin(), m.catalog.fullName(), run.id)},
	}
}

func (m *mock) catalogLog(branch string) (any, error) {
	sha, present := m.catalog.branches[branch]
	if !present {
		return nil, notFound()
	}
	log := []any{}
	for c := m.commits[sha]; c != nil; {
		author := c.author
		if author == nil {
			author = object{}
		}
		log = append(log, object{{"sha", c.sha}, {"message", c.message}, {"author", author}, {"date", c.date}})
		if len(c.parents) == 0 {
			break
		}
		c = m.commits[c.parents[0]]
	}
	return log, nil
}

func (m *mock) catalogRunsJSON() any {
	return listOf(m.git().runs, func(run *catalogRun) any {
		return object{{"workflow", run.workflow}, {"event", run.event}}
	})
}

func (m *mock) armRace(path string, data []byte) (any, error) {
	if path == "" {
		return nil, refusal(400, "path= names the file the catalog's workflow commits first")
	}
	g := m.git()
	g.armed, g.racePath, g.raceData = true, path, data
	return object{{"armed", true}, {"path", path}}, nil
}

// ageRelease moves a release and its assets back in time; an asset uploaded afterwards is newer than the release.
func (m *mock) ageRelease(idText, minutesText string) (any, error) {
	id, err := parseID(idText)
	if err != nil {
		return nil, err
	}
	if !isDigits(minutesText) {
		return nil, refusal(400, "minutes= takes a count")
	}
	r, err := m.releaseByID(id)
	if err != nil {
		return nil, err
	}
	shift := func(stamp string) string {
		moment, err := time.Parse("2006-01-02T15:04:05Z", stamp)
		if err != nil {
			return stamp
		}
		return moment.Add(-time.Duration(digitsValue(minutesText)) * time.Minute).Format("2006-01-02T15:04:05Z")
	}
	r.createdAt, r.publishedAt = shift(r.createdAt), shift(r.publishedAt)
	for _, a := range r.assets {
		a.createdAt = shift(a.createdAt)
	}
	return m.releaseJSON(r, false), nil
}
