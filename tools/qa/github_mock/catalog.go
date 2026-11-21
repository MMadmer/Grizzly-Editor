package main

import (
	"crypto/sha1"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"io"
	"maps"
	"slices"
	"strings"
	"unicode/utf8"
)

const (
	defaultBranch = "main"
	catalogID     = 2
	userID        = 1001
	// Issue ids on github.com no longer fit a signed 32-bit integer; an issue's number is added to this.
	issueIDBase    = 3000000000
	reopened       = "reopened"
	firstCommentID = 2150000001
	// The account GitHub Actions workflows comment as, with its id on github.com.
	actionsBot   = "github-actions[bot]"
	actionsBotID = 41898282
	// The contents API breaks its base64 after every 60 characters.
	contentLine = 60
)

var (
	issueStates = []string{"open", "closed"}
	// The reasons GitHub records when an issue is closed.
	closeReasons = []string{"completed", "not_planned"}
)

type account struct {
	login string
	id    int
	kind  string
}

func (a *account) json() any {
	if a == nil {
		return nil
	}
	return object{{"login", a.login}, {"id", a.id}, {"type", a.kind}}
}

type commit struct {
	sha     string
	files   map[string][]byte
	parents []string
	message string
	// What the Git Data API keeps of a commit (gitdata.go): who made it and when.
	author object
	date   string
}

type comment struct {
	id        int
	body      string
	user      *account
	createdAt string
}

type issue struct {
	number               int
	title, body          string
	user                 *account
	createdAt, updatedAt string
	// An entry standing for a pull request: GitHub lists those with the issues.
	pull                         bool
	state, stateReason, closedAt string
	closedBy                     *account
	// Oldest first.
	comments []*comment
}

// catalog is the Mod Browser catalog: branches over the mock's commit store, and issues numbered from 1.
type catalog struct {
	owner, name string
	branches    map[string]string
	issues      []*issue
}

func (c *catalog) fullName() string {
	return c.owner + "/" + c.name
}

func gitBlobSHA(data []byte) string {
	digest := sha1.New()
	fmt.Fprintf(digest, "blob %d\x00", len(data))
	digest.Write(data)
	return hex.EncodeToString(digest.Sum(nil))
}

func wrappedBase64(data []byte) string {
	text := base64.StdEncoding.EncodeToString(data)
	var wrapped strings.Builder
	for start := 0; start < len(text); start += contentLine {
		wrapped.WriteString(text[start:min(start+contentLine, len(text))])
		wrapped.WriteByte('\n')
	}
	return wrapped.String()
}

func harnessText(raw []byte, what string) (string, error) {
	if !utf8.Valid(raw) {
		return "", refusal(400, "the "+what+" is not UTF-8")
	}
	return string(raw), nil
}

// userFor is GitHub's user object: the account's unless login names another; an app's [bot] account is a Bot.
func (m *mock) userFor(login string) *account {
	lower := strings.ToLower(login)
	if lower == actionsBot {
		return &account{actionsBot, actionsBotID, "Bot"}
	}
	if lower == strings.ToLower(m.cfg.login) {
		return &account{m.cfg.login, userID, "User"}
	}
	sum := sha1.Sum([]byte(lower))
	kind := "User"
	if strings.HasSuffix(login, "[bot]") {
		kind = "Bot"
	}
	return &account{login, 2000000 + (int(sum[0])<<16 | int(sum[1])<<8 | int(sum[2])), kind}
}

func (m *mock) isAccount(user *account) bool {
	return user != nil && strings.EqualFold(user.login, m.cfg.login)
}

// isMaintainer: --login owns the catalog: push permission, and every issue is theirs to edit.
func (m *mock) isMaintainer() bool {
	return strings.EqualFold(m.cfg.login, m.catalog.owner)
}

func (m *mock) isCatalog(owner, repo string) bool {
	return strings.EqualFold(owner+"/"+repo, m.catalog.fullName())
}

func (m *mock) catalogJSON() object {
	c := m.catalog
	return object{
		{"id", catalogID},
		{"full_name", c.fullName()},
		{"name", c.name},
		{"owner", object{{"login", c.owner}}},
		{"default_branch", defaultBranch},
		{"private", false},
		{"fork", false},
		{"archived", false},
		{"permissions", object{{"push", m.isMaintainer()}, {"pull", true}}},
		{"html_url", m.origin() + "/" + c.fullName()},
	}
}

func (m *mock) seedCatalog(files map[string][]byte) {
	m.catalog.branches[defaultBranch] = m.newCommit(files, nil, "Initial commit").sha
}

func (m *mock) newCommit(files map[string][]byte, parents []string, message string) *commit {
	m.commitCount++
	digest := sha1.New()
	fmt.Fprintf(digest, "commit %d\n", m.commitCount)
	for _, parent := range parents {
		fmt.Fprintf(digest, "parent %s\n", parent)
	}
	for _, path := range slices.Sorted(maps.Keys(files)) {
		fmt.Fprintf(digest, "%s\x00%s\n", path, gitBlobSHA(files[path]))
	}
	io.WriteString(digest, message)
	result := &commit{sha: hex.EncodeToString(digest.Sum(nil)), files: files, parents: parents, message: message, date: timestamp()}
	m.commits[result.sha] = result
	return result
}

// snapshot is the files ?ref= names: a branch, refs/heads/<branch> or a commit.
func (m *mock) snapshot(ref string) (map[string][]byte, error) {
	sha := m.catalog.branches[strings.TrimPrefix(ref, "refs/heads/")]
	if _, known := m.commits[strings.ToLower(ref)]; sha == "" && known {
		sha = strings.ToLower(ref)
	}
	if sha == "" {
		return nil, refusal(404, "No commit found for the ref "+ref)
	}
	return m.commits[sha].files, nil
}

func (m *mock) getContents(path, ref string) (any, error) {
	files, err := m.snapshot(ref)
	if err != nil {
		return nil, err
	}
	data, present := files[path]
	if !present {
		return nil, notFound()
	}
	return object{
		{"type", "file"},
		{"name", path[strings.LastIndexByte(path, '/')+1:]},
		{"path", path},
		{"sha", gitBlobSHA(data)},
		{"size", len(data)},
		{"encoding", "base64"},
		{"content", wrappedBase64(data)},
	}, nil
}

// publicFile is public/<path> on the catalog's default branch: what the catalog publishes.
func (m *mock) publicFile(path string) ([]byte, error) {
	data, present := m.commits[m.catalog.branches[defaultBranch]].files["public/"+path]
	if !present {
		return nil, notFound()
	}
	return data, nil
}

func (m *mock) harnessWrite(path, branch string, data []byte) (any, error) {
	if path == "" {
		return nil, refusal(400, "path= names the file to write")
	}
	sha, present := m.catalog.branches[branch]
	if !present {
		return nil, notFound()
	}
	head := m.commits[sha]
	files := maps.Clone(head.files)
	files[path] = data
	result := m.newCommit(files, []string{head.sha}, "Update "+path)
	m.catalog.branches[branch] = result.sha
	return object{{"branch", branch}, {"path", path}, {"sha", gitBlobSHA(data)}, {"commit", result.sha}}, nil
}

func (m *mock) newIssue(login, title, body string, pull bool) *issue {
	now := timestamp()
	result := &issue{
		number: len(m.catalog.issues) + 1, title: title, body: body, user: m.userFor(login),
		createdAt: now, updatedAt: now, pull: pull, state: "open",
	}
	m.catalog.issues = append(m.catalog.issues, result)
	return result
}

func (m *mock) issueByNumber(number int) (*issue, error) {
	if number <= 0 || number > len(m.catalog.issues) {
		return nil, notFound()
	}
	return m.catalog.issues[number-1], nil
}

// issuePage is the issue on the web; a pull entry's page is the pull request's.
func (m *mock) issuePage(i *issue) string {
	kind := "issues"
	if i.pull {
		kind = "pull"
	}
	return fmt.Sprintf("%s/%s/%s/%d", m.origin(), m.catalog.fullName(), kind, i.number)
}

func (m *mock) catalogAPI() string {
	return m.origin() + "/repos/" + m.catalog.fullName()
}

func (m *mock) issueJSON(i *issue) object {
	api := m.catalogAPI()
	result := object{
		{"id", issueIDBase + i.number},
		{"number", i.number},
		{"title", i.title},
		// GitHub answers null for an issue without text.
		{"body", orNull(i.body)},
		{"state", i.state},
		{"state_reason", orNull(i.stateReason)},
		{"user", i.user.json()},
		{"html_url", m.issuePage(i)},
		{"url", fmt.Sprintf("%s/issues/%d", api, i.number)},
		{"comments", len(i.comments)},
		{"locked", false},
		{"created_at", i.createdAt},
		{"updated_at", i.updatedAt},
		{"closed_at", orNull(i.closedAt)},
		{"closed_by", i.closedBy.json()},
	}
	if i.pull {
		result = append(result, member{"pull_request", object{
			{"url", fmt.Sprintf("%s/pulls/%d", api, i.number)},
			{"html_url", m.issuePage(i)},
		}})
	}
	return result
}

func (m *mock) listIssues(query map[string]string) (any, error) {
	state := query["state"]
	if state == "" {
		state = "open"
	}
	if state != "open" && state != "closed" && state != "all" {
		return nil, validationFailed("Issue", "invalid", "state")
	}
	creator := strings.ToLower(query["creator"])
	matching := []*issue{}
	for _, i := range newestFirst(m.catalog.issues) {
		if (state == "all" || state == i.state) && (creator == "" || creator == strings.ToLower(i.user.login)) {
			matching = append(matching, i)
		}
	}
	return listOf(pageOf(matching, query), func(i *issue) any { return m.issueJSON(i) }), nil
}

// createIssue: POST issues, opened by the account the token belongs to.
func (m *mock) createIssue(body object) (any, error) {
	f := fields{body: body, resource: "Issue"}
	title := f.text("title", "")
	text := f.text("body", "")
	if strip(title) == "" {
		f.fail("missing_field", "title")
	}
	if f.err != nil {
		return nil, f.err
	}
	return m.issueJSON(m.newIssue(m.cfg.login, title, text, false)), nil
}

// updateIssue: PATCH issues/{n}, the author's own issue or any for the maintainer; an author reopens only what they closed.
func (m *mock) updateIssue(number int, body object) (any, error) {
	i, err := m.issueByNumber(number)
	if err != nil {
		return nil, err
	}
	if !m.isMaintainer() && !m.isAccount(i.user) {
		return nil, noAdminRights()
	}
	f := fields{body: body, resource: "Issue"}
	title := f.text("title", i.title)
	// "body": null clears the text.
	text := i.body
	if body.has("body") {
		text = f.text("body", "")
	}
	state := f.text("state", i.state)
	reason := f.optional("state_reason")
	if f.err != nil {
		return nil, f.err
	}
	if strip(title) == "" {
		return nil, validationFailed("Issue", "missing_field", "title")
	}
	if !slices.Contains(issueStates, state) {
		return nil, validationFailed("Issue", "invalid", "state")
	}
	// A reason counts only when the state changes, and then it has to fit the change.
	changing := state != i.state
	if reason != nil {
		fitting := []string{reopened}
		if state == "closed" {
			fitting = closeReasons
		}
		if !slices.Contains(closeReasons, *reason) && *reason != reopened || changing && !slices.Contains(fitting, *reason) {
			return nil, validationFailed("Issue", "invalid", "state_reason")
		}
	}
	if changing && state == "open" && !m.isMaintainer() && !m.isAccount(i.closedBy) {
		return nil, noAdminRights()
	}

	i.title, i.body = title, text
	switch {
	case changing && state == "closed":
		closing := closeReasons[0]
		if reason != nil {
			closing = *reason
		}
		m.closeIssue(i, m.cfg.login, closing)
	case changing:
		i.state, i.stateReason, i.closedAt, i.closedBy = "open", reopened, "", nil
	}
	i.updatedAt = timestamp()
	return m.issueJSON(i), nil
}

func (m *mock) closeIssue(i *issue, login, reason string) {
	i.state, i.stateReason, i.closedBy = "closed", reason, m.userFor(login)
	i.closedAt = timestamp()
	i.updatedAt = i.closedAt
}

// harnessClose: the catalog's workflow - or whoever login names - closes an open issue.
func (m *mock) harnessClose(number int, reason, login string) (any, error) {
	i, err := m.issueByNumber(number)
	if err != nil {
		return nil, err
	}
	if !slices.Contains(closeReasons, reason) {
		return nil, refusal(400, "reason= takes completed or not_planned")
	}
	if i.state != "open" {
		return nil, refusal(422, fmt.Sprintf("Issue #%d is not open", number))
	}
	m.closeIssue(i, login, reason)
	return m.issueJSON(i), nil
}

// plantIssue: an issue opened by anyone; pull=1 makes it an entry standing for a pull request.
func (m *mock) plantIssue(creator, title string, raw []byte, pull string) (any, error) {
	if creator == "" || strip(title) == "" {
		return nil, refusal(400, "creator= and title= name the issue's author and its title")
	}
	if pull != "" && pull != "0" && pull != "1" {
		return nil, refusal(400, "pull= takes 1 or 0")
	}
	text, err := harnessText(raw, "issue's body")
	if err != nil {
		return nil, err
	}
	return m.issueJSON(m.newIssue(creator, title, text, pull == "1")), nil
}

func (m *mock) commentJSON(i *issue, c *comment) object {
	api := m.catalogAPI()
	return object{
		{"id", c.id},
		{"url", fmt.Sprintf("%s/issues/comments/%d", api, c.id)},
		{"html_url", fmt.Sprintf("%s#issuecomment-%d", m.issuePage(i), c.id)},
		{"issue_url", fmt.Sprintf("%s/issues/%d", api, i.number)},
		{"user", c.user.json()},
		{"body", c.body},
		{"created_at", c.createdAt},
		{"updated_at", c.createdAt},
	}
}

func (m *mock) listComments(number int, query map[string]string) (any, error) {
	i, err := m.issueByNumber(number)
	if err != nil {
		return nil, err
	}
	return listOf(pageOf(i.comments, query), func(c *comment) any { return m.commentJSON(i, c) }), nil
}

func (m *mock) addComment(i *issue, login, text string) object {
	c := &comment{m.nextCommentID, text, m.userFor(login), timestamp()}
	m.nextCommentID++
	i.comments = append(i.comments, c)
	i.updatedAt = c.createdAt
	return m.commentJSON(i, c)
}

// postComment: POST issues/{n}/comments, written by the account the token belongs to, whatever the issue's state.
func (m *mock) postComment(number int, body object) (any, error) {
	i, err := m.issueByNumber(number)
	if err != nil {
		return nil, err
	}
	f := fields{body: body, resource: "IssueComment"}
	text := f.text("body", "")
	if text == "" {
		f.fail("missing_field", "body")
	}
	if f.err != nil {
		return nil, f.err
	}
	return m.addComment(i, m.cfg.login, text), nil
}

// plantComments: count comments by anyone - the catalog's workflow unless author names somebody else.
func (m *mock) plantComments(number int, author string, raw []byte, countText string) (any, error) {
	i, err := m.issueByNumber(number)
	if err != nil {
		return nil, err
	}
	text, err := harnessText(raw, "comment's body")
	if err != nil {
		return nil, err
	}
	if text == "" {
		return nil, refusal(400, "the request body is the comment's text")
	}
	if !isDigits(countText) || digitsValue(countText) < 1 || digitsValue(countText) > 1000 {
		return nil, refusal(400, "count= takes 1 to 1000")
	}
	planted := make([]any, digitsValue(countText))
	for index := range planted {
		planted[index] = m.addComment(i, author, text)
	}
	return planted, nil
}

func (m *mock) branchJSON(sha string) object {
	source := m.commits[sha]
	files, binary := object{}, object{}
	for _, path := range slices.Sorted(maps.Keys(source.files)) {
		if data := source.files[path]; utf8.Valid(data) {
			files = append(files, member{path, string(data)})
		} else {
			binary = append(binary, member{path, base64.StdEncoding.EncodeToString(data)})
		}
	}
	return object{{"sha", sha}, {"message", source.message}, {"files", files}, {"files_base64", binary}}
}

func (m *mock) catalogStateJSON() object {
	branches := object{}
	for _, name := range slices.Sorted(maps.Keys(m.catalog.branches)) {
		branches = append(branches, member{name, m.branchJSON(m.catalog.branches[name])})
	}
	return object{
		{"repositories", object{{m.catalog.fullName(), object{{"branches", branches}}}}},
		{"issues", listOf(m.catalog.issues, func(i *issue) any {
			comments := listOf(i.comments, func(c *comment) any { return m.commentJSON(i, c) })
			return append(m.issueJSON(i), member{"comment_list", comments})
		})},
	}
}
