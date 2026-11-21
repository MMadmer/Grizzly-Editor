package main

import (
	"cmp"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"regexp"
	"runtime/debug"
	"slices"
	"strconv"
	"strings"
	"time"
)

const (
	// Named as a comment in test_github_mock.py names it: bodies are read in blocks of exactly this size.
	READ_CHUNK    = 1 << 20
	socketTimeout = 120 * time.Second
	serverName    = "GrizzlyGitHubMock/1.0"
)

var (
	handledMethods  = []string{"GET", "POST", "PUT", "PATCH", "DELETE"}
	lowerSHA256     = regexp.MustCompile(`^[0-9a-f]{64}$`)
	errClientClosed = errors.New("the client closed the connection mid-body")
)

// answer is sent as JSON when payload is set, as body otherwise.
type answer struct {
	status  int
	payload any
	body    []byte
	headers map[string]string
	// cutting: only the first cut bytes of the body are sent, then the connection is held for
	// hold and reset (an asset download the harness breaks).
	cutting bool
	cut     int
	hold    time.Duration
	// sent learns how many body bytes went out.
	sent func(int)
}

func reply(status int, payload any, err error) (*answer, error) {
	if err != nil {
		return nil, err
	}
	return &answer{status: status, payload: payload}, nil
}

func ok(payload any, err error) (*answer, error) {
	return reply(http.StatusOK, payload, err)
}

func created(payload any, err error) (*answer, error) {
	return reply(http.StatusCreated, payload, err)
}

func noContent(err error) (*answer, error) {
	return reply(http.StatusNoContent, nil, err)
}

func redirect(location string, err error) (*answer, error) {
	if err != nil {
		return nil, err
	}
	return &answer{status: http.StatusFound, headers: map[string]string{"Location": location}}, nil
}

func text(body []byte, kind string) *answer {
	return &answer{status: http.StatusOK, body: body, headers: map[string]string{"Content-Type": kind}}
}

// exchange is the transport of one request: parse, read the body, ask the mock, answer, log.
type exchange struct {
	mock              *mock
	w                 http.ResponseWriter
	r                 *http.Request
	control           *http.ResponseController
	target, path      string
	segments          []string
	query             map[string]string
	bodyLeft, bytesIn int64
	shutdownRequested bool
	hungUp            bool
}

func (m *mock) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	if !slices.Contains(handledMethods, r.Method) {
		refuseMethod(w, r)
		return
	}
	x := newExchange(m, w, r)
	response := x.dispatch()
	// Status 0 in the log: the connection ended without a response.
	status := 0
	if response != nil && x.send(response) == nil {
		status = response.status
	}
	x.record(status)
	if status == 0 {
		x.hangUp(false)
	}
	if x.shutdownRequested {
		select {
		case m.stop <- struct{}{}:
		default:
		}
	}
}

func refuseMethod(w http.ResponseWriter, r *http.Request) {
	page := fmt.Sprintf("<!DOCTYPE HTML>\n<title>Error response</title>\n<p>Error code: 501</p>\n<p>Message: Unsupported method (%s).</p>\n", pyRepr(r.Method))
	header := w.Header()
	header.Set("Server", serverName)
	header.Set("Connection", "close")
	header.Set("Content-Type", "text/html;charset=utf-8")
	header.Set("Content-Length", strconv.Itoa(len(page)))
	w.WriteHeader(http.StatusNotImplemented)
	io.WriteString(w, page)
}

func newExchange(m *mock, w http.ResponseWriter, r *http.Request) *exchange {
	target := latin1(r.RequestURI)
	if strings.HasPrefix(target, "//") {
		target = "/" + strings.TrimLeft(target, "/")
	}
	path, query := splitTarget(target, r.URL)
	segments := []string{}
	for part := range strings.SplitSeq(path, "/") {
		if part != "" {
			segments = append(segments, unquote(part))
		}
	}
	return &exchange{
		mock: m, w: w, r: r, control: http.NewResponseController(w),
		target: target, path: path, segments: segments, query: parseQuery(query, true),
		bodyLeft: max(r.ContentLength, 0),
	}
}

func splitTarget(target string, parsed *url.URL) (path, query string) {
	if !strings.HasPrefix(target, "/") {
		return parsed.EscapedPath(), parsed.RawQuery
	}
	target, _, _ = strings.Cut(target, "#")
	path, query, _ = strings.Cut(target, "?")
	return path, query
}

func (x *exchange) dispatch() (response *answer) {
	// A mock defect must reach the client as a visible failure.
	defer func() {
		if failure := recover(); failure != nil {
			fmt.Fprintf(os.Stderr, "%v\n%s\n", failure, debug.Stack())
			response = &answer{status: 500, payload: object{{"message", fmt.Sprintf("Mock failure: %v", failure)}}}
		}
	}()
	result, err := x.route()
	var refused *apiError
	if errors.As(err, &refused) {
		return &answer{status: refused.status, payload: refused.payload, headers: refused.headers}
	}
	return result
}

// ---- plumbing ---------------------------------------------------------

// readBlock fills block as a blocking file read does: a clean end of stream leaves what arrived, a socket error loses it.
func (x *exchange) readBlock(block []byte) (int, error) {
	filled := 0
	for filled < len(block) {
		x.control.SetReadDeadline(time.Now().Add(socketTimeout))
		count, err := x.r.Body.Read(block[filled:])
		filled += count
		switch {
		case filled == len(block):
			return filled, nil
		case err == io.EOF || errors.Is(err, io.ErrUnexpectedEOF):
			return filled, errClientClosed
		case err != nil:
			return 0, err
		}
	}
	return filled, nil
}

// readBody consumes count body bytes in blocks, paced to rate bytes per second when given.
func (x *exchange) readBody(count int64, sink func([]byte), rate int) error {
	if count <= 0 {
		return nil
	}
	blockSize := int64(READ_CHUNK)
	if rate != 0 {
		blockSize = max(1, min(READ_CHUNK, int64(rate)/10))
	}
	block := make([]byte, min(blockSize, count))
	started := time.Now()
	done := int64(0)
	for done < count {
		size, err := x.readBlock(block[:min(blockSize, count-done)])
		done += int64(size)
		x.bodyLeft -= int64(size)
		x.bytesIn += int64(size)
		if sink != nil && size > 0 {
			sink(block[:size])
		}
		if err != nil {
			return err
		}
		if rate != 0 {
			if ahead := time.Duration(float64(done)/float64(rate)*float64(time.Second)) - time.Since(started); ahead > 0 {
				time.Sleep(ahead)
			}
		}
	}
	return nil
}

func (x *exchange) readSmallBody() ([]byte, error) {
	raw := []byte{}
	err := x.readBody(x.bodyLeft, func(chunk []byte) { raw = append(raw, chunk...) }, 0)
	return raw, err
}

func jsonBody(raw []byte) (object, error) {
	value, valid := parseJSON(raw)
	body, isObject := value.(object)
	if !valid || !isObject {
		return nil, refusal(400, "Problems parsing JSON")
	}
	return body, nil
}

func (x *exchange) formBody(raw []byte) (object, error) {
	if strings.Contains(x.r.Header.Get("Content-Type"), "json") {
		return jsonBody(raw)
	}
	form := object{}
	for key, value := range parseQuery(replaceInvalid(string(raw)), false) {
		form = append(form, member{key, value})
	}
	return form, nil
}

func (x *exchange) send(response *answer) error {
	// Closing a socket that still holds unread input resets it and can destroy the response.
	if err := x.readBody(x.bodyLeft, nil, 0); err != nil {
		return err
	}
	body := response.body
	if response.payload != nil {
		body = dumps(response.payload)
	}
	header := x.w.Header()
	header.Set("Server", serverName)
	if response.payload != nil {
		header.Set("Content-Type", "application/json; charset=utf-8")
	}
	for key, value := range response.headers {
		header.Set(key, value)
	}
	// RFC 7230 3.3.2: a 204 carries no Content-Length.
	if response.status != http.StatusNoContent {
		header.Set("Content-Length", strconv.Itoa(len(body)))
	}
	header.Set("Connection", "close")
	x.control.SetWriteDeadline(time.Now().Add(socketTimeout))
	x.w.WriteHeader(response.status)
	if response.cutting && response.cut < len(body) {
		written, _ := x.w.Write(body[:response.cut])
		x.control.Flush()
		if response.sent != nil {
			response.sent(written)
		}
		if response.hold > 0 {
			time.Sleep(response.hold)
		}
		// a tidy close: the client gets every byte sent, then a body shorter than it was told
		// (a reset could throw away what it had not read yet)
		x.hangUp(false)
		return errClientClosed
	}
	written, err := x.w.Write(body)
	if response.sent != nil {
		response.sent(written)
	}
	if err != nil {
		return err
	}
	return x.control.Flush()
}

// hangUp ends the connection without a response. A reset (linger 0) makes the client see an error, not a tidy end of stream.
func (x *exchange) hangUp(reset bool) {
	if x.hungUp {
		return
	}
	connection, _, err := x.control.Hijack()
	if err != nil {
		return
	}
	x.hungUp = true
	if tcp, isTCP := connection.(*net.TCPConn); isTCP && reset {
		tcp.SetLinger(0)
	}
	connection.Close()
}

func (x *exchange) record(status int) {
	if len(x.segments) > 0 && strings.HasPrefix(x.segments[0], "__") {
		return
	}
	m := x.mock
	_, authorized := x.r.Header["Authorization"]
	_, vtKey := x.r.Header["X-Apikey"]
	m.mu.Lock()
	defer m.mu.Unlock()
	entry := object{
		{"n", len(m.requests) + 1},
		{"method", x.r.Method},
		{"path", x.target},
		{"status", status},
		{"authorized", authorized},
		{"vt_key", vtKey},
		{"bytes_in", x.bytesIn},
	}
	m.requests = append(m.requests, entry)
	if m.logFile != nil {
		stamp := time.Now().UTC().Format("2006-01-02T15:04:05.000-07:00")
		m.logFile.Write(append(dumps(append(slices.Clip(entry), member{"time", stamp})), '\n'))
	}
}

func (x *exchange) checkAPIHeaders() error {
	if x.r.Header.Get("User-Agent") == "" {
		return refusal(403, "Request forbidden by administrative rules. Please make sure your request has a User-Agent header.")
	}
	return x.mock.checkToken(x.r.Header.Get("Authorization"))
}

// ---- routes -----------------------------------------------------------

// match reports whether segments spell pattern, where "*" stands for any one segment.
func match(segments []string, pattern ...string) bool {
	if len(segments) != len(pattern) {
		return false
	}
	for i, want := range pattern {
		if want != "*" && want != segments[i] {
			return false
		}
	}
	return true
}

// route answers nil when the connection was dropped, on purpose or by the client.
func (x *exchange) route() (*answer, error) {
	if len(x.segments) > 0 && x.segments[0] == "upload" {
		return x.routeUpload()
	}
	if len(x.segments) > 0 && x.segments[0] == "vt" {
		return x.routeVT()
	}
	raw, err := x.readSmallBody()
	if err != nil {
		return nil, err
	}
	x.mock.mu.Lock()
	defer x.mock.mu.Unlock()
	return x.routeLocked(raw)
}

func (x *exchange) routeLocked(raw []byte) (*answer, error) {
	m, seg, query := x.mock, x.segments, x.query
	get, post := x.r.Method == "GET", x.r.Method == "POST"
	if len(seg) > 0 && (seg[0] == "user" || seg[0] == "repos") && !x.anonymousRead() {
		if err := x.checkAPIHeaders(); err != nil {
			return nil, err
		}
	}

	switch {
	case post && match(seg, "api", "v1", "reports"):
		return m.takeReport(x.r.Header, raw)
	case get && match(seg, "__reports"):
		return ok(m.reportsJSON(), nil)
	case post && match(seg, "__reports", "fail"):
		return ok(m.failReports(query["status"], query["count"]))
	case get && match(seg, "__state"):
		return ok(m.stateJSON(), nil)
	case post && match(seg, "__shutdown"):
		x.shutdownRequested = true
		return ok(object{{"ok", true}}, nil)
	case get && match(seg, "__catalog"):
		return ok(m.catalogStateJSON(), nil)
	case post && match(seg, "__catalog", "file"):
		return ok(m.harnessWrite(strings.Trim(query["path"], "/"), cmp.Or(query["branch"], defaultBranch), raw))
	case get && match(seg, "__catalog", "log"):
		return ok(m.catalogLog(cmp.Or(query["branch"], defaultBranch)))
	case get && match(seg, "__catalog", "runs"):
		return ok(m.catalogRunsJSON(), nil)
	case post && match(seg, "__catalog", "race"):
		return ok(m.armRace(strings.Trim(query["path"], "/"), raw))
	case post && match(seg, "__release", "age"):
		return ok(m.ageRelease(query["id"], query["minutes"]))
	case post && match(seg, "__catalog", "issue"):
		return ok(m.plantIssue(query["creator"], query["title"], raw, query["pull"]))
	case post && match(seg, "__catalog", "comment"):
		number, err := parseID(query["number"])
		if err != nil {
			return nil, err
		}
		return ok(m.plantComments(number, cmp.Or(query["author"], actionsBot), raw, cmp.Or(query["count"], "1")))
	case post && match(seg, "__catalog", "close"):
		number, err := parseID(query["number"])
		if err != nil {
			return nil, err
		}
		return ok(m.harnessClose(number, cmp.Or(query["reason"], closeReasons[0]), cmp.Or(query["by"], actionsBot)))
	case match(seg, "__asset", "*"):
		return m.assetHarness(seg[1], x.r.Method, query)
	case get && match(seg, "__vt"):
		return ok(m.vtStateJSON(), nil)
	case post && match(seg, "__vt", "queue"):
		sha := strings.ToLower(query["sha"])
		lookups, given := query["lookups"]
		if !given {
			lookups = "1"
		}
		if !lowerSHA256.MatchString(sha) || !isDigits(lookups) || digitsValue(lookups) < 1 {
			return nil, refusal(400, "sha= takes a SHA-256 and lookups= a count from 1")
		}
		m.vtQueue(sha, digitsValue(lookups))
		return ok(m.vtStateJSON(), nil)
	case post && match(seg, "__vt", "forget"):
		if err := m.vtForget(strings.ToLower(query["sha"])); err != nil {
			return nil, err
		}
		return ok(m.vtStateJSON(), nil)
	case post && match(seg, "__vt", "seed"):
		body, err := jsonBody(raw)
		if err != nil {
			return nil, err
		}
		seed, err := readVtSeed(body)
		if err != nil {
			return nil, refusal(400, err.Error())
		}
		m.vtApplySeed(seed)
		return ok(m.vtStateJSON(), nil)

	case get && match(seg, "user"):
		return ok(m.userFor(m.cfg.login).json(), nil)
	case len(seg) >= 3 && seg[0] == "repos" && m.isRepo(seg[1], seg[2]):
		if err := m.checkRepo(seg[1], seg[2], !get); err != nil {
			return nil, err
		}
		return x.routeRepo(seg[3:], raw)
	case len(seg) >= 3 && seg[0] == "repos" && m.isCatalog(seg[1], seg[2]):
		return x.routeCatalog(seg[3:], raw)
	case get && len(seg) > 2 && seg[0] == "catalog" && seg[1] == "public":
		data, err := m.publicFile(strings.Join(seg[2:], "/"))
		if err != nil {
			return nil, err
		}
		if strings.HasSuffix(strings.ToLower(seg[len(seg)-1]), ".dds") {
			return text(data, "application/octet-stream"), nil
		}
		return text(data, "text/plain; charset=utf-8"), nil

	case post && match(seg, "login", "device", "code"):
		form, err := x.formBody(raw)
		if err != nil {
			return nil, err
		}
		return ok(m.deviceCode(form))
	case post && match(seg, "login", "oauth", "access_token"):
		form, err := x.formBody(raw)
		if err != nil {
			return nil, err
		}
		return ok(m.deviceToken(form), nil)
	case get && match(seg, "login", "device"):
		page := "GitHub mock: device verification page. Approval is driven by the mock's command line.\n"
		return text([]byte(page), "text/plain; charset=utf-8"), nil

	case get && match(seg, "cdn", "*", "*", "*"):
		releaseID, err := parseID(seg[1])
		if err != nil {
			return nil, err
		}
		assetID, err := parseID(seg[2])
		if err != nil {
			return nil, err
		}
		a, err := m.cdnAsset(releaseID, assetID, seg[3])
		if err != nil {
			return nil, err
		}
		return x.cdnAnswer(a)
	case get && match(seg, "*", "*", "releases", "latest", "download", "*"):
		return redirect(m.latestDownload(seg[0], seg[1], seg[5]))
	case get && match(seg, "*", "*", "releases", "download", "*", "*"):
		return redirect(m.tagDownload(seg[0], seg[1], seg[4], seg[5]))
	}
	return nil, notFound()
}

func (x *exchange) routeRepo(rest []string, raw []byte) (*answer, error) {
	m, method := x.mock, x.r.Method
	switch {
	case method == "GET" && len(rest) == 0:
		return ok(m.repoJSON(), nil)
	case method == "GET" && match(rest, "releases"):
		return ok(m.listReleases(x.query), nil)
	case method == "POST" && match(rest, "releases"):
		body, err := jsonBody(raw)
		if err != nil {
			return nil, err
		}
		return created(m.createRelease(body))
	case method == "GET" && match(rest, "releases", "latest"):
		r, err := m.latestRelease()
		if err != nil {
			return nil, err
		}
		return ok(m.releaseJSON(r, false), nil)
	case method == "GET" && match(rest, "releases", "tags", "*"):
		r := m.publishedByTag(rest[2], nil)
		if r == nil {
			return nil, notFound()
		}
		return ok(m.releaseJSON(r, false), nil)
	case method == "DELETE" && match(rest, "releases", "assets", "*"):
		id, err := parseID(rest[2])
		if err != nil {
			return nil, err
		}
		return noContent(m.deleteAsset(id))
	case method == "GET" && match(rest, "releases", "*"):
		r, err := x.releaseFrom(rest[1])
		if err != nil {
			return nil, err
		}
		return ok(m.releaseJSON(r, false), nil)
	case method == "PATCH" && match(rest, "releases", "*"):
		id, err := parseID(rest[1])
		if err != nil {
			return nil, err
		}
		body, err := jsonBody(raw)
		if err != nil {
			return nil, err
		}
		return ok(m.updateRelease(id, body))
	case method == "DELETE" && match(rest, "releases", "*"):
		id, err := parseID(rest[1])
		if err != nil {
			return nil, err
		}
		return noContent(m.deleteRelease(id))
	case method == "GET" && match(rest, "releases", "*", "assets"):
		r, err := x.releaseFrom(rest[1])
		if err != nil {
			return nil, err
		}
		return ok(listOf(r.assets, func(a *asset) any { return m.assetJSON(r, a, false) }), nil)
	case len(rest) > 3 && match(rest[:3], "git", "refs", "tags") && (method == "GET" || method == "DELETE"):
		tag := strings.Join(rest[3:], "/")
		if method == "DELETE" {
			return noContent(m.deleteTag(tag))
		}
		if !m.tags[tag] {
			return nil, notFound()
		}
		return ok(object{{"ref", "refs/tags/" + tag}}, nil)
	}
	return nil, notFound()
}

func (x *exchange) releaseFrom(text string) (*release, error) {
	id, err := parseID(text)
	if err != nil {
		return nil, err
	}
	return x.mock.releaseByID(id)
}

// routeCatalog: the Mod Browser catalog, its repository, its files, its issues and their comments.
func (x *exchange) routeCatalog(rest []string, raw []byte) (*answer, error) {
	m, method := x.mock, x.r.Method
	if result, handled, err := x.routeCatalogGit(rest, raw); handled {
		return result, err
	}
	switch {
	case method == "GET" && len(rest) == 0:
		return ok(m.catalogJSON(), nil)
	case method == "GET" && len(rest) > 1 && rest[0] == "contents":
		return ok(m.getContents(strings.Join(rest[1:], "/"), cmp.Or(x.query["ref"], defaultBranch)))
	case method == "GET" && match(rest, "issues"):
		return ok(m.listIssues(x.query))
	case method == "POST" && match(rest, "issues"):
		body, err := jsonBody(raw)
		if err != nil {
			return nil, err
		}
		return created(m.createIssue(body))
	case method == "GET" && match(rest, "issues", "*"):
		number, err := parseID(rest[1])
		if err != nil {
			return nil, err
		}
		i, err := m.issueByNumber(number)
		if err != nil {
			return nil, err
		}
		return ok(m.issueJSON(i), nil)
	case method == "PATCH" && match(rest, "issues", "*"):
		number, err := parseID(rest[1])
		if err != nil {
			return nil, err
		}
		body, err := jsonBody(raw)
		if err != nil {
			return nil, err
		}
		return ok(m.updateIssue(number, body))
	case method == "GET" && match(rest, "issues", "*", "comments"):
		number, err := parseID(rest[1])
		if err != nil {
			return nil, err
		}
		return ok(m.listComments(number, x.query))
	case method == "POST" && match(rest, "issues", "*", "comments"):
		number, err := parseID(rest[1])
		if err != nil {
			return nil, err
		}
		body, err := jsonBody(raw)
		if err != nil {
			return nil, err
		}
		return created(m.postComment(number, body))
	}
	return nil, notFound()
}

// routeVT: VirusTotal's API under /vt/api/v3, and the upload URLs it hands out under /vt/upload.
func (x *exchange) routeVT() (*answer, error) {
	m, seg := x.mock, x.segments
	get, post := x.r.Method == "GET", x.r.Method == "POST"
	m.mu.Lock()
	err := m.checkVT(x.r.Header)
	m.mu.Unlock()
	if err != nil {
		return nil, err
	}

	switch {
	case post && match(seg, "vt", "api", "v3", "files"):
		return x.vtUpload(int64(m.cfg.vtMaxUpload), "; a larger file goes to an upload URL (GET /files/upload_url)")
	case post && match(seg, "vt", "upload", "*"):
		m.mu.Lock()
		err := m.spendUploadURL(seg[2])
		m.mu.Unlock()
		if err != nil {
			return nil, err
		}
		return x.vtUpload(vtUploadURLLimit, "")
	case get && match(seg, "vt", "api", "v3", "files", "upload_url"):
		m.mu.Lock()
		defer m.mu.Unlock()
		return ok(object{{"data", m.vtUploadURL()}}, nil)
	case get && match(seg, "vt", "api", "v3", "files", "*"):
		m.mu.Lock()
		defer m.mu.Unlock()
		return ok(m.vtFileJSON(seg[4]))
	case get && match(seg, "vt", "api", "v3", "analyses", "*"):
		m.mu.Lock()
		defer m.mu.Unlock()
		return ok(m.vtAnalysisJSON(seg[4]))
	}
	return nil, vtError(404, "NotFoundError", fmt.Sprintf("No VirusTotal route %s %s", x.r.Method, x.path))
}

// vtUpload streams a multipart upload: only the file's size and SHA-256 are kept.
func (x *exchange) vtUpload(limit int64, hint string) (*answer, error) {
	if x.bodyLeft > limit {
		return nil, vtError(413, "RequestEntityTooLargeError", fmt.Sprintf("The request body is larger than %d bytes%s", limit, hint))
	}
	boundary, err := multipartBoundary(x.r.Header.Get("Content-Type"))
	if err != nil {
		return nil, err
	}
	form := newMultipartFile(boundary)
	if err := x.readBody(x.bodyLeft, form.feed, 0); err != nil {
		return nil, err
	}
	sha, size, name, err := form.finish()
	if err != nil {
		return nil, err
	}
	x.mock.mu.Lock()
	defer x.mock.mu.Unlock()
	return ok(x.mock.vtRegister(sha, size, name), nil)
}

func (x *exchange) routeUpload() (*answer, error) {
	if seg := x.segments; x.r.Method == "POST" && match(seg, "upload", "repos", "*", "*", "releases", "*", "assets") {
		return x.upload(seg[2], seg[3], seg[5], x.query["name"])
	}
	if err := x.checkAPIHeaders(); err != nil {
		return nil, err
	}
	return nil, notFound()
}

func (x *exchange) upload(owner, repo, releaseID, name string) (*answer, error) {
	m := x.mock
	contentType := cmp.Or(latin1(x.r.Header.Get("Content-Type")), "application/octet-stream")
	rate := m.cfg.throttleKbps * 125

	checkAccess := func() error {
		if err := x.checkAPIHeaders(); err != nil {
			return err
		}
		return m.checkRepo(owner, repo, true)
	}
	// GitHub keeps a stub of an upload that died, and the stub owns the name - if it would have accepted the upload.
	leaveStarter := func() {
		m.mu.Lock()
		defer m.mu.Unlock()
		if id, err := parseID(releaseID); err == nil && checkAccess() == nil {
			m.addAsset(id, name, contentType, []byte{}, "", "starter")
		}
	}

	m.mu.Lock()
	m.uploads++
	dropping := m.uploads == m.cfg.dropUpload
	m.mu.Unlock()
	if dropping {
		if err := x.readBody(x.bodyLeft/2, nil, rate); err != nil {
			return nil, err
		}
		leaveStarter()
		x.hangUp(true)
		return nil, nil
	}

	var data []byte
	digest := sha256.New()
	err := x.readBody(x.bodyLeft, func(chunk []byte) {
		data = append(data, chunk...)
		digest.Write(chunk)
	}, rate)
	if err != nil {
		leaveStarter()
		return nil, err
	}

	m.mu.Lock()
	defer m.mu.Unlock()
	if err := checkAccess(); err != nil {
		return nil, err
	}
	if _, declared := x.r.Header["Content-Length"]; !declared {
		return nil, refusal(411, "Length Required")
	}
	if len(data) == 0 {
		return nil, refusal(400, "Bad Content-Length: 0")
	}
	id, err := parseID(releaseID)
	if err != nil {
		return nil, err
	}
	return created(m.addAsset(id, name, contentType, data, hex.EncodeToString(digest.Sum(nil)), "uploaded"))
}

func (x *exchange) cdnAnswer(a *asset) (*answer, error) {
	headers := map[string]string{"Content-Type": "application/octet-stream", "Accept-Ranges": "bytes"}
	asked := x.r.Header.Get("Range")
	first, last, ranged, err := parseRange(asked, len(a.data))
	if err != nil {
		return nil, err
	}
	var result *answer
	if !ranged || x.mock.noRanges {
		first, last = 0, len(a.data)-1
		result = &answer{status: http.StatusOK, body: a.data, headers: headers}
	} else {
		headers["Content-Range"] = fmt.Sprintf("bytes %d-%d/%d", first, last, len(a.data))
		result = &answer{status: http.StatusPartialContent, body: a.data[first : last+1], headers: headers}
	}
	x.mock.assetServed(a.name, asked, first, last, result)
	return result, nil
}

// parseRange resolves a single bytes range; ranged is false for no header or a form this mock ignores.
func parseRange(header string, total int) (first, last int, ranged bool, err error) {
	unit, spec, _ := strings.Cut(header, "=")
	firstText, lastText, dash := strings.Cut(strip(spec), "-")
	if strip(unit) != "bytes" || !dash || strings.Contains(spec, ",") {
		return 0, 0, false, nil
	}
	if firstText != "" {
		if !isDigits(firstText) || lastText != "" && !isDigits(lastText) {
			return 0, 0, false, nil
		}
		first, last = digitsValue(firstText), total-1
		if lastText != "" {
			end := digitsValue(lastText)
			if end < first {
				return 0, 0, false, nil
			}
			last = min(end, total-1)
		}
	} else {
		// bytes=-n: the last n bytes.
		if !isDigits(lastText) {
			return 0, 0, false, nil
		}
		first, last = max(0, total-digitsValue(lastText)), total-1
	}
	if first > last {
		return 0, 0, false, &apiError{
			status:  http.StatusRequestedRangeNotSatisfiable,
			payload: object{{"message", "Range Not Satisfiable"}},
			headers: map[string]string{"Content-Range": fmt.Sprintf("bytes */%d", total)},
		}
	}
	return first, last, true, nil
}
