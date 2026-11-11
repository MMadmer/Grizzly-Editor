package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"hash"
	"net/http"
	"regexp"
	"slices"
	"strings"
	"time"
)

const (
	vtUploadURLLimit = 681574400
	partHeadersLimit = 16384
	// Named as a comment in test_mod_catalog.py names it.
	VT_RATE_WINDOW = 60 * time.Second
)

var (
	vtCategories = []string{"malicious", "suspicious", "undetected", "harmless", "type-unsupported", "timeout"}
	vtDetected   = []string{"malicious", "suspicious"}
	sha256Text   = regexp.MustCompile(`^[0-9a-fA-F]{64}$`)
	headerParam  = regexp.MustCompile(`;\s*([^\s=;]+)\s*=\s*("(?:[^"\\]|\\.)*"|[^;]*)`)
	quotedPair   = regexp.MustCompile(`\\(.)`)
)

func vtError(status int, code, message string) *apiError {
	return &apiError{status: status, payload: object{{"error", object{{"code", code}, {"message", message}}}}}
}

// readVtResults checks {"<engine>": "<category>"} against the categories VirusTotal uses.
func readVtResults(value any, source string) (object, error) {
	results, isObject := value.(object)
	if !isObject {
		return nil, fmt.Errorf("%s: expected a JSON object of engine names", source)
	}
	for _, entry := range results {
		if category, isText := entry.value.(string); !isText || !slices.Contains(vtCategories, category) {
			return nil, fmt.Errorf("%s: %s has the category %s, not one of %s",
				source, entry.key, pyRepr(entry.value), strings.Join(vtCategories, ", "))
		}
	}
	return results, nil
}

// readVtSeed checks {"<sha256>": {"<engine>": "<category>"}} and puts the hashes in lower case.
func readVtSeed(value any) (object, error) {
	files, isObject := value.(object)
	if !isObject {
		return nil, fmt.Errorf("expected a JSON object keyed by SHA-256")
	}
	seed := object{}
	for _, entry := range files {
		if !sha256Text.MatchString(entry.key) {
			return nil, fmt.Errorf("%s is not a SHA-256", pyRepr(entry.key))
		}
		results, err := readVtResults(entry.value, entry.key)
		if err != nil {
			return nil, err
		}
		seed = seed.set(strings.ToLower(entry.key), results)
	}
	return seed, nil
}

func vtStats(results object) object {
	stats := make(object, len(vtCategories))
	for i, category := range vtCategories {
		count := 0
		for _, entry := range results {
			if entry.value == category {
				count++
			}
		}
		stats[i] = member{category, count}
	}
	return stats
}

type vtFile struct {
	sha256  string
	size    int64
	results object
	// Lookups that will still find the report queued.
	queued       int
	uploads      int
	analysisDate int64
	names        []string
}

type vtAnalysis struct {
	id, sha256 string
	size       int64
	// Set when the file's report is out; nil while it is queued.
	stats object
}

func (m *mock) checkVT(headers http.Header) error {
	if _, leaked := headers["Authorization"]; leaked {
		return vtError(400, "BadRequestError", "Authorization header sent to VirusTotal")
	}
	if keys := headers["X-Apikey"]; len(keys) == 0 || keys[0] != m.cfg.vtKey {
		return vtError(401, "WrongCredentialsError", "Wrong API key")
	}
	if m.cfg.vtRate == 0 {
		return nil
	}
	now := time.Now()
	window := VT_RATE_WINDOW
	if m.cfg.vtRateWindowMs > 0 {
		window = time.Duration(m.cfg.vtRateWindowMs) * time.Millisecond
	}
	for len(m.vtCalls) > 0 && now.Sub(m.vtCalls[0]) >= window {
		m.vtCalls = m.vtCalls[1:]
	}
	if len(m.vtCalls) >= m.cfg.vtRate {
		return vtError(429, "QuotaExceededError", "Quota exceeded")
	}
	m.vtCalls = append(m.vtCalls, now)
	return nil
}

func (m *mock) vtFileBySHA(sha string) *vtFile {
	for _, entry := range m.vtFiles {
		if entry.sha256 == sha {
			return entry
		}
	}
	return nil
}

// vtEnsure is the file's entry, made with these size and results when VirusTotal did not know it.
func (m *mock) vtEnsure(sha string, size int64, results object) *vtFile {
	entry := m.vtFileBySHA(sha)
	if entry == nil {
		entry = &vtFile{sha256: sha, size: size, results: results}
		m.vtFiles = append(m.vtFiles, entry)
	}
	return entry
}

func (m *mock) vtApplySeed(seed object) {
	for _, item := range seed {
		entry := m.vtEnsure(item.key, 0, object{})
		entry.results, entry.queued = item.value.(object), 0
		m.vtComplete(entry)
	}
}

// vtComplete: the report is out, and the file's queued analyses complete with its verdicts.
func (m *mock) vtComplete(entry *vtFile) {
	entry.analysisDate = time.Now().Unix()
	for _, analysis := range m.vtAnalyses {
		if analysis.sha256 == entry.sha256 && analysis.stats == nil {
			analysis.stats = vtStats(entry.results)
		}
	}
}

// vtLookup is one lookup of a report; true means it is still queued and this lookup counted down.
func (m *mock) vtLookup(entry *vtFile) bool {
	if entry.queued <= 0 {
		return false
	}
	entry.queued--
	if entry.queued == 0 {
		m.vtComplete(entry)
	}
	return true
}

// vtQueue: the file is being analysed - known or not - and its report stays out for lookups lookups.
func (m *mock) vtQueue(sha string, lookups int) {
	m.vtEnsure(sha, 0, m.cfg.vtUploadResults).queued = lookups
}

// vtForget: VirusTotal no longer knows the file, nor any analysis of it.
func (m *mock) vtForget(sha string) error {
	if m.vtFileBySHA(sha) == nil {
		return notFound()
	}
	m.vtFiles = slices.DeleteFunc(m.vtFiles, func(entry *vtFile) bool { return entry.sha256 == sha })
	m.vtAnalyses = slices.DeleteFunc(m.vtAnalyses, func(item *vtAnalysis) bool { return item.sha256 == sha })
	return nil
}

func (m *mock) vtFileJSON(fileID string) (any, error) {
	missing := vtError(404, "NotFoundError", fmt.Sprintf(`File "%s" not found`, fileID))
	entry := m.vtFileBySHA(strings.ToLower(fileID))
	if entry == nil {
		return nil, missing
	}
	attributes := object{{"sha256", entry.sha256}, {"size", entry.size}}
	firstAnalysis := entry.analysisDate == 0
	if m.vtLookup(entry) {
		if firstAnalysis && m.cfg.vtHideNew {
			return nil, missing
		}
		attributes = append(attributes, member{"last_analysis_results", object{}}, member{"last_analysis_stats", vtStats(nil)})
	} else {
		results := object{}
		for _, item := range entry.results {
			category := item.value.(string)
			var detection any
			if slices.Contains(vtDetected, category) {
				detection = "Trojan.Generic"
			}
			results = append(results, member{item.key, object{{"category", category}, {"engine_name", item.key}, {"result", detection}}})
		}
		attributes = append(attributes,
			member{"last_analysis_date", entry.analysisDate},
			member{"last_analysis_results", results},
			member{"last_analysis_stats", vtStats(entry.results)})
	}
	link := m.origin() + "/vt/api/v3/files/" + entry.sha256
	return object{{"data", object{
		{"id", entry.sha256},
		{"type", "file"},
		{"links", object{{"self", link}}},
		{"attributes", attributes},
	}}}, nil
}

// vtRegister is an upload: the file becomes known, or stays known with its verdicts, and its report queues.
func (m *mock) vtRegister(sha string, size int64, name string) object {
	entry := m.vtEnsure(sha, size, m.cfg.vtUploadResults)
	entry.size, entry.uploads, entry.queued = size, entry.uploads+1, m.cfg.vtQueue
	if name != "" && !slices.Contains(entry.names, name) {
		entry.names = append(entry.names, name)
	}
	stamp := fmt.Sprintf("%s:%d:%d", sha[:32], time.Now().Unix(), len(m.vtAnalyses)+1)
	analysis := &vtAnalysis{id: base64.URLEncoding.EncodeToString([]byte(stamp)), sha256: sha, size: size}
	if at := slices.IndexFunc(m.vtAnalyses, func(item *vtAnalysis) bool { return item.id == analysis.id }); at >= 0 {
		m.vtAnalyses[at] = analysis
	} else {
		m.vtAnalyses = append(m.vtAnalyses, analysis)
	}
	if entry.queued == 0 {
		m.vtComplete(entry)
	}
	link := m.origin() + "/vt/api/v3/analyses/" + analysis.id
	return object{{"data", object{{"type", "analysis"}, {"id", analysis.id}, {"links", object{{"self", link}}}}}}
}

func (m *mock) vtUploadURL() string {
	token := base64.RawURLEncoding.EncodeToString(randomBytes(24))
	m.vtUploadTokens[token] = true
	return m.origin() + "/vt/upload/" + token
}

func (m *mock) spendUploadURL(token string) error {
	if !m.vtUploadTokens[token] {
		return vtError(404, "NotFoundError", "This upload URL does not exist or was already used")
	}
	delete(m.vtUploadTokens, token)
	return nil
}

func (m *mock) vtAnalysisJSON(id string) (any, error) {
	at := slices.IndexFunc(m.vtAnalyses, func(item *vtAnalysis) bool { return item.id == id })
	if at < 0 {
		return nil, vtError(404, "NotFoundError", fmt.Sprintf(`Analysis "%s" not found`, id))
	}
	analysis := m.vtAnalyses[at]
	entry := m.vtFileBySHA(analysis.sha256)
	status, stats := "queued", vtStats(nil)
	if analysis.stats != nil || !m.vtLookup(entry) {
		status, stats = "completed", analysis.stats
		if stats == nil {
			stats = vtStats(entry.results)
		}
	}
	return object{
		{"data", object{{"id", analysis.id}, {"type", "analysis"}, {"attributes", object{{"status", status}, {"stats", stats}}}}},
		{"meta", object{{"file_info", object{{"sha256", analysis.sha256}, {"size", analysis.size}}}}},
	}, nil
}

func (m *mock) vtStateJSON() object {
	files := object{}
	for _, entry := range m.vtFiles {
		files = append(files, member{entry.sha256, object{
			{"sha256", entry.sha256},
			{"size", entry.size},
			{"results", entry.results},
			{"queued", entry.queued},
			{"uploads", entry.uploads},
			{"names", listOf(entry.names, func(name string) any { return name })},
		}})
	}
	analyses := listOf(m.vtAnalyses, func(item *vtAnalysis) any {
		status := "completed"
		if item.stats == nil {
			status = "queued"
		}
		return object{{"id", item.id}, {"sha256", item.sha256}, {"status", status}}
	})
	return object{{"files", files}, {"analyses", analyses}}
}

// headerParams turns `form-data; name="file"` into "form-data" and {"name": "file"}, quotes removed.
func headerParams(value string) (string, map[string]string) {
	main, rest, _ := strings.Cut(value, ";")
	params := map[string]string{}
	for _, found := range headerParam.FindAllStringSubmatch(";"+rest, -1) {
		text := strip(found[2])
		if len(text) >= 2 && text[0] == '"' && text[len(text)-1] == '"' {
			text = quotedPair.ReplaceAllString(text[1:len(text)-1], "$1")
		}
		params[strings.ToLower(found[1])] = text
	}
	return strings.ToLower(strip(main)), params
}

func multipartBoundary(contentType string) (string, error) {
	kind, params := headerParams(contentType)
	boundary := params["boundary"]
	if kind != "multipart/form-data" || boundary == "" || len(boundary) > 70 {
		return "", vtError(400, "BadRequestError", "Expected a multipart/form-data body with a boundary")
	}
	return boundary, nil
}

// multipartFile streams a multipart/form-data body that must hold exactly one part, the field "file"
// with a filename, and keeps only that part's size and SHA-256. A malformed body is read to its end
// all the same: the error waits for finish.
type multipartFile struct {
	opening, delimiter, buffer []byte
	state                      string
	parts                      int
	problem                    string
	digest                     hash.Hash
	size                       int64
	filename                   string
}

func newMultipartFile(boundary string) *multipartFile {
	opening := []byte("--" + boundary)
	return &multipartFile{opening: opening, delimiter: append([]byte("\r\n"), opening...), state: "start", digest: sha256.New()}
}

func (p *multipartFile) fail(problem string) {
	p.problem, p.state, p.buffer = problem, "failed", nil
}

func (p *multipartFile) feed(chunk []byte) {
	if p.state != "failed" && p.state != "closed" {
		p.buffer = append(p.buffer, chunk...)
		for p.step() {
		}
	}
}

// step moves the parse on; false when it needs more input or has stopped.
func (p *multipartFile) step() bool {
	buffer := p.buffer
	switch p.state {
	case "start":
		if len(buffer) < len(p.opening) {
			return false
		}
		// Without a preamble the body opens with the boundary itself.
		if bytes.HasPrefix(buffer, p.opening) {
			p.buffer = buffer[len(p.opening):]
			p.state = "delimiter"
		} else {
			p.state = "preamble"
		}
		return true
	case "preamble":
		at := bytes.Index(buffer, p.delimiter)
		if at < 0 {
			// Keep what may be the start of the delimiter.
			p.buffer = buffer[max(0, len(buffer)-len(p.delimiter)+1):]
			return false
		}
		p.buffer = buffer[at+len(p.delimiter):]
		p.state = "delimiter"
		return true
	case "delimiter":
		// Transport padding, then CRLF opens a part and "--" closes the body.
		padded := bytes.TrimLeft(buffer, " \t")
		if len(padded) < 2 {
			return false
		}
		switch mark := string(padded[:2]); {
		case mark == "--" && p.parts > 0:
			p.state = "closed"
		case mark == "--":
			p.fail("the body closes before its first part")
		case mark != "\r\n":
			p.fail("a boundary line is malformed")
		case p.parts > 0:
			p.fail("the body holds more than one part")
		default:
			p.buffer = padded[2:]
			p.state = "headers"
			return true
		}
		return false
	case "headers":
		var lines []string
		skip := 2
		if !bytes.HasPrefix(buffer, []byte("\r\n")) {
			at := bytes.Index(buffer, []byte("\r\n\r\n"))
			if at < 0 {
				if len(buffer) > partHeadersLimit {
					p.fail("the part's headers do not end")
				}
				return false
			}
			lines, skip = strings.Split(replaceInvalid(string(buffer[:at])), "\r\n"), at+4
		}
		if !p.readHeaders(lines) {
			return false
		}
		p.buffer = buffer[skip:]
		p.parts++
		p.state = "data"
		return true
	case "data":
		at := bytes.Index(buffer, p.delimiter)
		end := at
		if at < 0 {
			end = len(buffer) - len(p.delimiter) + 1
		}
		if end > 0 {
			p.digest.Write(buffer[:end])
			p.size += int64(end)
			buffer = buffer[end:]
			p.buffer = buffer
		}
		if at < 0 {
			return false
		}
		p.buffer = buffer[len(p.delimiter):]
		p.state = "delimiter"
		return true
	}
	return false
}

func (p *multipartFile) readHeaders(lines []string) bool {
	headers := map[string]string{}
	for _, line := range lines {
		name, value, colon := strings.Cut(line, ":")
		if !colon || strip(name) == "" {
			p.fail("a part header is malformed: " + pyRepr(line))
			return false
		}
		headers[strings.ToLower(strip(name))] = strip(value)
	}
	kind, params := headerParams(headers["content-disposition"])
	filename, named := params["filename"]
	if kind != "form-data" || params["name"] != "file" || !named {
		p.fail(`the part is not Content-Disposition: form-data; name="file"; filename="..."`)
		return false
	}
	p.filename = filename
	return true
}

// finish answers the SHA-256, size and filename of the file part, or a 400 for a malformed body.
func (p *multipartFile) finish() (string, int64, string, error) {
	if p.problem == "" && p.state != "closed" {
		p.problem = "the body ends before its closing boundary"
	}
	if p.problem != "" {
		return "", 0, "", vtError(400, "BadRequestError", "Malformed multipart/form-data body: "+p.problem)
	}
	return hex.EncodeToString(p.digest.Sum(nil)), p.size, p.filename, nil
}
