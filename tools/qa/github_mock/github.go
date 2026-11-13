package main

import (
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"maps"
	"os"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"
)

const (
	firstReleaseID = 250000001
	firstAssetID   = 310000001
	deviceGrant    = "urn:ietf:params:oauth:grant-type:device_code"
	// Timer jitter on the client must not turn an honest poll into slow_down.
	pollGrace = 100 * time.Millisecond
)

var makeLatestValues = []string{"true", "false", "legacy"}

// apiError is an error answer: a status plus a JSON payload in GitHub's shape.
type apiError struct {
	status  int
	payload any
	headers map[string]string
}

func (e *apiError) Error() string {
	return "HTTP " + strconv.Itoa(e.status)
}

func refusal(status int, message string) *apiError {
	return &apiError{status: status, payload: object{{"message", message}}}
}

func notFound() *apiError {
	return refusal(404, "Not Found")
}

func validationFailed(resource, code, field string) *apiError {
	errors := []any{object{{"resource", resource}, {"code", code}, {"field", field}}}
	return &apiError{status: 422, payload: object{{"message", "Validation Failed"}, {"errors", errors}}}
}

func noAdminRights() *apiError {
	return refusal(403, "Must have admin rights to Repository.")
}

// fields reads a JSON request body: the first invalid field stays in err and every later read does nothing.
type fields struct {
	body     object
	resource string
	err      error
}

func (f *fields) fail(code, key string) {
	if f.err == nil {
		f.err = validationFailed(f.resource, code, key)
	}
}

func (f *fields) flag(key string, fallback bool) bool {
	value, present := f.body.get(key)
	if !present || f.err != nil {
		return fallback
	}
	flag, isFlag := value.(bool)
	if !isFlag {
		f.fail("invalid", key)
	}
	return flag
}

// optional is nil for a key that is absent or null.
func (f *fields) optional(key string) *string {
	value, _ := f.body.get(key)
	if value == nil || f.err != nil {
		return nil
	}
	text, isText := value.(string)
	if !isText {
		f.fail("invalid", key)
		return nil
	}
	return &text
}

func (f *fields) text(key, fallback string) string {
	if text := f.optional(key); text != nil {
		return *text
	}
	return fallback
}

func timestamp() string {
	return time.Now().UTC().Format("2006-01-02T15:04:05Z")
}

func randomBytes(count int) []byte {
	data := make([]byte, count)
	rand.Read(data)
	return data
}

func sha256Hex(data []byte) string {
	sum := sha256.Sum256(data)
	return hex.EncodeToString(sum[:])
}

type asset struct {
	id                int
	name, contentType string
	data              []byte
	sha256, state     string
}

type release struct {
	id                               int
	tagName, name, body              string
	draft, prerelease                bool
	untagged, createdAt, publishedAt string
	assets                           []*asset
}

// slug is the tag as the web URLs spell it: a draft has no tag there yet.
func (r *release) slug() string {
	if r.draft {
		return "untagged-" + r.untagged
	}
	return r.tagName
}

func (r *release) eligible() bool {
	return !r.draft && !r.prerelease
}

type deviceEntry struct {
	polls int
	last  time.Time
	scope any
}

// mock holds the repositories, VirusTotal's files and every rule applied to them. Callers hold mu.
type mock struct {
	cfg     *config
	mu      sync.Mutex
	stop    chan struct{}
	port    int
	logFile *os.File

	releases []*release
	tags     map[string]bool
	latestID int
	// CDN lag: the web download path serves staleID for staleLeft more requests.
	staleID, staleLeft         int
	nextReleaseID, nextAssetID int
	uploads                    int
	requests                   []any
	deviceCodes                map[string]*deviceEntry

	catalog       *catalog
	commits       map[string]*commit
	commitCount   int
	nextCommentID int

	vtFiles        []*vtFile
	vtAnalyses     []*vtAnalysis
	vtUploadTokens map[string]bool
	// When the requests that count toward --vt-rate arrived.
	vtCalls []time.Time

	// The release assets as a client reads them (assets.go): the harness's faults and counts.
	assetDrop  *assetDrop
	noRanges   bool
	assetStats map[string]*assetStat
}

func newMock(cfg *config) *mock {
	catalogOwner, catalogName, _ := strings.Cut(cfg.catalogRepo, "/")
	return &mock{
		cfg:            cfg,
		stop:           make(chan struct{}, 1),
		tags:           map[string]bool{},
		nextReleaseID:  firstReleaseID,
		nextAssetID:    firstAssetID,
		requests:       []any{},
		deviceCodes:    map[string]*deviceEntry{},
		catalog:        &catalog{owner: catalogOwner, name: catalogName, branches: map[string]string{}},
		commits:        map[string]*commit{},
		nextCommentID:  firstCommentID,
		vtUploadTokens: map[string]bool{},
		assetStats:     map[string]*assetStat{},
	}
}

func (m *mock) origin() string {
	return "http://127.0.0.1:" + strconv.Itoa(m.port)
}

// ---- access ---------------------------------------------------------

func (m *mock) checkToken(authorization string) error {
	scheme, token, _ := strings.Cut(authorization, " ")
	if strings.ToLower(scheme) != "bearer" || strip(token) != m.cfg.token {
		return &apiError{status: 401, payload: object{{"message", "Bad credentials"}, {"status", "401"}}}
	}
	return nil
}

func (m *mock) isRepo(owner, repo string) bool {
	return strings.EqualFold(owner+"/"+repo, m.cfg.repo)
}

func (m *mock) checkRepo(owner, repo string, writing bool) error {
	switch {
	case !m.isRepo(owner, repo):
		return notFound()
	case writing && m.cfg.archived:
		return refusal(403, "Repository was archived so is read-only.")
	case writing && m.cfg.noPush:
		return refusal(403, "Resource not accessible by personal access token")
	}
	return nil
}

// ---- JSON shapes ----------------------------------------------------

func (m *mock) repoJSON() object {
	canPush := !m.cfg.noPush
	return object{
		{"id", 1},
		{"full_name", m.cfg.repo},
		{"private", m.cfg.private},
		{"archived", m.cfg.archived},
		{"default_branch", "main"},
		{"permissions", object{{"admin", canPush}, {"push", canPush}, {"pull", true}}},
	}
}

func (m *mock) reportedDigest(a *asset) any {
	if a.state != "uploaded" {
		return nil
	}
	value := a.sha256
	if m.cfg.corruptDigest != "" && strings.HasSuffix(a.name, m.cfg.corruptDigest) {
		value = sha256Hex([]byte(value))
	}
	return "sha256:" + value
}

func (m *mock) assetJSON(r *release, a *asset, withSHA256 bool) object {
	result := object{
		{"id", a.id},
		{"name", a.name},
		{"size", len(a.data)},
		{"state", a.state},
		{"content_type", a.contentType},
	}
	if !m.cfg.noDigest {
		result = append(result, member{"digest", m.reportedDigest(a)})
	}
	url := fmt.Sprintf("%s/%s/releases/download/%s/%s", m.origin(), m.cfg.repo, quote(r.slug()), quote(a.name))
	result = append(result, member{"browser_download_url", url})
	if withSHA256 {
		result = append(result, member{"sha256", a.sha256})
	}
	return result
}

func (m *mock) releaseJSON(r *release, withSHA256 bool) object {
	return object{
		{"id", r.id},
		{"tag_name", r.tagName},
		{"name", r.name},
		{"body", r.body},
		{"draft", r.draft},
		{"prerelease", r.prerelease},
		{"created_at", r.createdAt},
		{"published_at", orNull(r.publishedAt)},
		{"html_url", fmt.Sprintf("%s/%s/releases/tag/%s", m.origin(), m.cfg.repo, quote(r.slug()))},
		{"upload_url", fmt.Sprintf("%s/upload/repos/%s/releases/%d/assets{?name,label}", m.origin(), m.cfg.repo, r.id)},
		{"assets", listOf(r.assets, func(a *asset) any { return m.assetJSON(r, a, withSHA256) })},
	}
}

func (m *mock) stateJSON() object {
	tags := slices.Sorted(maps.Keys(m.tags))
	return object{
		{"latest_id", orNull(m.latestID)},
		{"releases", listOf(newestFirst(m.releases), func(r *release) any { return m.releaseJSON(r, true) })},
		{"tags", listOf(tags, func(tag string) any { return tag })},
		{"requests", slices.Clone(m.requests)},
		{"uploads", m.uploads},
	}
}

// ---- releases -------------------------------------------------------

func (m *mock) newRelease(tag, name, body string, draft, prerelease bool) *release {
	r := &release{
		id: m.nextReleaseID, tagName: tag, name: name, body: body, draft: draft, prerelease: prerelease,
		untagged: hex.EncodeToString(randomBytes(10)), createdAt: timestamp(),
	}
	m.nextReleaseID++
	m.releases = append(m.releases, r)
	m.markPublished(r)
	return r
}

// markPublished: a release that is out has a publication time and a git tag; the tag outlives the release.
func (m *mock) markPublished(r *release) {
	if r.draft {
		r.publishedAt = ""
		return
	}
	if r.publishedAt == "" {
		r.publishedAt = timestamp()
	}
	m.tags[r.tagName] = true
}

func (m *mock) deleteTag(tag string) error {
	if !m.tags[tag] {
		return refusal(422, "Reference does not exist")
	}
	if m.cfg.protectTags {
		return refusal(422, "Repository rule violations found\n\nCannot delete this protected ref.")
	}
	delete(m.tags, tag)
	// GitHub does not drop a release whose tag goes away: it falls back to a draft.
	if r := m.publishedByTag(tag, nil); r != nil {
		r.draft = true
		r.publishedAt = ""
		if m.latestID == r.id {
			m.setLatest(m.newestEligibleID())
		}
	}
	return nil
}

func (m *mock) newAsset(r *release, name, contentType string, data []byte, digest, state string) *asset {
	if digest == "" {
		digest = sha256Hex(data)
	}
	a := &asset{m.nextAssetID, name, contentType, data, digest, state}
	m.nextAssetID++
	r.assets = append(r.assets, a)
	return a
}

// seedReleases pre-creates releases in file order, so the last entry is the newest. Seeding never arms the CDN lag.
func (m *mock) seedReleases(value any) error {
	entries, isList := value.([]any)
	if !isList {
		return fmt.Errorf("expected a JSON list of releases")
	}
	for _, item := range entries {
		entry, isObject := item.(object)
		if !isObject {
			return fmt.Errorf("a release is not a JSON object")
		}
		tag, hasTag := entry.text("tag_name")
		if !hasTag {
			return fmt.Errorf("a release has no tag_name")
		}
		name, _ := entry.text("name")
		body, _ := entry.text("body")
		draft, _ := entry.get("draft")
		prerelease, _ := entry.get("prerelease")
		r := m.newRelease(tag, name, body, truthy(draft), truthy(prerelease))
		assets, _ := entry.get("assets")
		list, _ := assets.(object)
		for _, seeded := range list {
			text, isText := seeded.value.(string)
			if !isText {
				return fmt.Errorf("the asset %s of %s is not a text", seeded.key, tag)
			}
			m.newAsset(r, seeded.key, "application/octet-stream", []byte(text), "", "uploaded")
		}
		if latest, _ := entry.get("latest"); truthy(latest) {
			m.latestID = r.id
		}
	}
	return nil
}

func (m *mock) releaseByID(id int) (*release, error) {
	for _, r := range m.releases {
		if r.id == id {
			return r, nil
		}
	}
	return nil, notFound()
}

func (m *mock) publishedByTag(tag string, besides *release) *release {
	for _, r := range m.releases {
		if !r.draft && r.tagName == tag && r != besides {
			return r
		}
	}
	return nil
}

func (m *mock) latestRelease() (*release, error) {
	if m.latestID == 0 {
		return nil, notFound()
	}
	return m.releaseByID(m.latestID)
}

func (m *mock) setLatest(id int) {
	if id == m.latestID {
		return
	}
	if id != 0 && m.cfg.staleLatest > 0 {
		m.staleID, m.staleLeft = m.latestID, m.cfg.staleLatest
	}
	m.latestID = id
}

func (m *mock) newestEligibleID() int {
	newest := 0
	for _, r := range m.releases {
		if r.eligible() {
			newest = max(newest, r.id)
		}
	}
	return newest
}

func (m *mock) listReleases(query map[string]string) []any {
	page := pageOf(newestFirst(m.releases), query)
	return listOf(page, func(r *release) any { return m.releaseJSON(r, false) })
}

func readMakeLatest(body object, draft, prerelease bool) (string, error) {
	text := "legacy"
	if value, present := body.get("make_latest"); present {
		text, _ = value.(string)
	}
	if !slices.Contains(makeLatestValues, text) || text == "true" && (draft || prerelease) {
		return "", validationFailed("Release", "invalid", "make_latest")
	}
	return text, nil
}

func (m *mock) createRelease(body object) (any, error) {
	if m.cfg.empty {
		return nil, refusal(422, "Repository is empty.")
	}
	f := fields{body: body, resource: "Release"}
	tag := f.text("tag_name", "")
	if tag == "" {
		f.fail("missing_field", "tag_name")
	}
	draft := f.flag("draft", false)
	prerelease := f.flag("prerelease", false)
	if f.err != nil {
		return nil, f.err
	}
	makeLatest, err := readMakeLatest(body, draft, prerelease)
	if err != nil {
		return nil, err
	}
	if m.publishedByTag(tag, nil) != nil {
		return nil, validationFailed("Release", "already_exists", "tag_name")
	}
	name := f.text("name", "")
	text := f.text("body", "")
	if f.err != nil {
		return nil, f.err
	}

	r := m.newRelease(tag, name, text, draft, prerelease)
	if makeLatest == "true" || makeLatest == "legacy" && r.eligible() {
		m.setLatest(r.id)
	}
	return m.releaseJSON(r, false), nil
}

func (m *mock) updateRelease(id int, body object) (any, error) {
	r, err := m.releaseByID(id)
	if err != nil {
		return nil, err
	}
	f := fields{body: body, resource: "Release"}
	tag := f.text("tag_name", r.tagName)
	draft := f.flag("draft", r.draft)
	prerelease := f.flag("prerelease", r.prerelease)
	if f.err != nil {
		return nil, f.err
	}
	makeLatest, err := readMakeLatest(body, draft, prerelease)
	if err != nil {
		return nil, err
	}
	if !body.has("make_latest") && m.cfg.latestOnEdit {
		// "Default: true" is what GitHub documents for an update too: an edit that says nothing about the mark takes it.
		makeLatest = "false"
		if !draft && !prerelease {
			makeLatest = "true"
		}
	}
	if tag == "" {
		return nil, validationFailed("Release", "missing_field", "tag_name")
	}
	// A draft may sit on a published tag it already had; taking the tag or publishing onto it is refused.
	if (!draft || tag != r.tagName) && m.publishedByTag(tag, r) != nil {
		return nil, validationFailed("Release", "already_exists", "tag_name")
	}
	name := f.text("name", r.name)
	text := f.text("body", r.body)
	if f.err != nil {
		return nil, f.err
	}

	wasEligible := r.eligible()
	r.tagName, r.name, r.body = tag, name, text
	r.draft, r.prerelease = draft, prerelease
	m.markPublished(r)
	if makeLatest == "true" || makeLatest == "legacy" && r.eligible() && !wasEligible {
		m.setLatest(r.id)
	} else if m.latestID == r.id && !r.eligible() {
		m.setLatest(m.newestEligibleID())
	}
	return m.releaseJSON(r, false), nil
}

func (m *mock) deleteRelease(id int) error {
	r, err := m.releaseByID(id)
	if err != nil {
		return err
	}
	m.releases = slices.DeleteFunc(m.releases, func(item *release) bool { return item == r })
	if m.latestID == id {
		m.setLatest(m.newestEligibleID())
	}
	return nil
}

// ---- assets ---------------------------------------------------------

func (m *mock) addAsset(id int, name, contentType string, data []byte, digest, state string) (any, error) {
	r, err := m.releaseByID(id)
	if err != nil {
		return nil, err
	}
	if name == "" {
		return nil, validationFailed("ReleaseAsset", "missing_field", "name")
	}
	if m.cfg.renameAssets {
		name = strings.ReplaceAll(name, "-", ".")
	}
	if slices.ContainsFunc(r.assets, func(a *asset) bool { return a.name == name }) {
		return nil, validationFailed("ReleaseAsset", "already_exists", "name")
	}
	return m.assetJSON(r, m.newAsset(r, name, contentType, data, digest, state), false), nil
}

func (m *mock) deleteAsset(id int) error {
	for _, r := range m.releases {
		for i, a := range r.assets {
			if a.id == id {
				r.assets = slices.Delete(r.assets, i, i+1)
				return nil
			}
		}
	}
	return notFound()
}

// ---- anonymous downloads ---------------------------------------------

func (m *mock) cdnLocation(r *release, name string) (string, error) {
	if r != nil {
		for _, a := range r.assets {
			if a.name == name && a.state == "uploaded" {
				return fmt.Sprintf("/cdn/%d/%d/%s", r.id, a.id, quote(a.name)), nil
			}
		}
	}
	return "", notFound()
}

func (m *mock) latestDownload(owner, repo, name string) (string, error) {
	if !m.isRepo(owner, repo) || m.cfg.private {
		return "", notFound()
	}
	id := m.latestID
	if m.staleLeft > 0 {
		m.staleLeft--
		id = m.staleID
	}
	var r *release
	if id != 0 {
		var err error
		if r, err = m.releaseByID(id); err != nil {
			return "", err
		}
	}
	return m.cdnLocation(r, name)
}

func (m *mock) tagDownload(owner, repo, tag, name string) (string, error) {
	if !m.isRepo(owner, repo) || m.cfg.private {
		return "", notFound()
	}
	return m.cdnLocation(m.publishedByTag(tag, nil), name)
}

func (m *mock) cdnAsset(releaseID, assetID int, name string) (*asset, error) {
	r, err := m.releaseByID(releaseID)
	if err != nil {
		return nil, err
	}
	for _, a := range r.assets {
		if a.id == assetID && a.name == name && a.state == "uploaded" {
			return a, nil
		}
	}
	return nil, notFound()
}

// ---- device flow ----------------------------------------------------

func (m *mock) deviceCode(form object) (any, error) {
	if id, isText := form.text("client_id"); !isText || id != m.cfg.clientID {
		return nil, &apiError{status: 404, payload: object{{"error", "Not Found"}}}
	}
	code := "dc-" + hex.EncodeToString(randomBytes(20))
	scope, _ := form.get("scope")
	if !truthy(scope) {
		scope = "public_repo"
	}
	m.deviceCodes[code] = &deviceEntry{scope: scope}
	return object{
		{"device_code", code},
		{"user_code", "WDJB-MJHT"},
		{"verification_uri", m.origin() + "/login/device"},
		{"expires_in", 900},
		{"interval", m.cfg.deviceInterval},
	}, nil
}

func deviceError(code string) object {
	return object{{"error", code}}
}

func (m *mock) deviceToken(form object) object {
	if id, isText := form.text("client_id"); !isText || id != m.cfg.clientID {
		return deviceError("incorrect_client_credentials")
	}
	if grant, isText := form.text("grant_type"); !isText || grant != deviceGrant {
		return deviceError("unsupported_grant_type")
	}
	code, _ := form.text("device_code")
	entry := m.deviceCodes[code]
	if entry == nil {
		return deviceError("incorrect_device_code")
	}

	// Every poll restarts the clock, so a client that ignores slow_down never gets its token.
	now := time.Now()
	tooFast := !entry.last.IsZero() && now.Sub(entry.last) < time.Duration(m.cfg.deviceInterval)*time.Second-pollGrace
	entry.last = now
	if tooFast {
		return object{{"error", "slow_down"}, {"interval", m.cfg.deviceInterval + 5}}
	}

	entry.polls++
	if entry.polls < m.cfg.deviceApproveAfter {
		return deviceError("authorization_pending")
	}
	delete(m.deviceCodes, code)
	if m.cfg.deviceDeny {
		return deviceError("access_denied")
	}
	return object{{"access_token", m.cfg.token}, {"token_type", "bearer"}, {"scope", entry.scope}}
}
