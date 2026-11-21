package main

// The report hub as the editor's feedback speaks to it (docs/REPORTS.md, section 5): one multipart POST
// of a report, held here for the harness to read (GET /__reports, the package as base64), and the
// hub's busy answers on request (POST /__reports/fail?status=503&count=1).

import (
	"bytes"
	"cmp"
	"encoding/base64"
	"errors"
	"io"
	"mime"
	"mime/multipart"
	"net/http"
	"strings"
	"unicode/utf8"
)

// The key a QA run's editor sends unless GRIZZLY_REPORT_TOKEN says another (the editor's default).
const qaReportToken = "qa-report-token"

type hubReport struct {
	id       int
	fields   map[string]string
	key      string
	fileName string
	zip      []byte
	agent    string
}

type reportDesk struct {
	reports []*hubReport
	// statuses to answer the next reports with, oldest first
	fails []int
	tried int
}

func hubError(status int, code, message string) *apiError {
	return &apiError{status: status, payload: object{{"error", object{{"code", code}, {"message", message}}}}}
}

func (m *mock) desk() *reportDesk {
	if m.reportDesk == nil {
		m.reportDesk = &reportDesk{}
	}
	return m.reportDesk
}

// takeReport is POST /api/v1/reports, checked the way the hub checks it.
func (m *mock) takeReport(header http.Header, raw []byte) (*answer, error) {
	d := m.desk()
	d.tried++
	if len(d.fails) > 0 {
		status := d.fails[0]
		d.fails = d.fails[1:]
		return nil, hubError(status, "upload_busy", "Too many uploads are in progress")
	}
	if header.Get("Authorization") != "Bearer "+qaReportToken {
		return nil, hubError(401, "unauthorized", "Invalid upload token")
	}
	mediaType, params, err := mime.ParseMediaType(header.Get("Content-Type"))
	if err != nil || mediaType != "multipart/form-data" || params["boundary"] == "" {
		return nil, hubError(415, "invalid_content_type", "multipart/form-data is required")
	}
	r := &hubReport{fields: map[string]string{}, key: header.Get("Idempotency-Key"), agent: header.Get("User-Agent")}
	reader := multipart.NewReader(bytes.NewReader(raw), params["boundary"])
	for {
		part, err := reader.NextPart()
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return nil, hubError(400, "invalid_request", "read multipart data: "+err.Error())
		}
		data, err := io.ReadAll(part)
		if err != nil {
			return nil, hubError(400, "invalid_request", "read a field: "+err.Error())
		}
		name := part.FormName()
		if _, twice := r.fields[name]; twice || (name == "report" && r.zip != nil) {
			return nil, hubError(400, "invalid_request", "field "+name+" was submitted more than once")
		}
		switch name {
		case "title", "description", "version", "product", "kind":
			r.fields[name] = string(data)
		case "report":
			r.fileName, r.zip = part.FileName(), data
		default:
			return nil, hubError(400, "invalid_request", "unexpected multipart field "+name)
		}
	}
	switch {
	case strings.TrimSpace(r.fields["title"]) == "" || utf8.RuneCountInString(r.fields["title"]) > 200:
		return nil, hubError(400, "invalid_request", "title is required")
	case strings.TrimSpace(r.fields["description"]) == "" || utf8.RuneCountInString(r.fields["description"]) > 10000:
		return nil, hubError(400, "invalid_request", "description is required")
	case r.fields["product"] != "" && r.fields["product"] != "dead-air-refined" && r.fields["product"] != "grizzly-editor":
		return nil, hubError(400, "invalid_request", "product must be dead-air-refined or grizzly-editor")
	case r.fields["kind"] != "" && r.fields["kind"] != "bug" && r.fields["kind"] != "suggestion":
		return nil, hubError(400, "invalid_request", "kind must be bug or suggestion")
	case len(r.zip) > 5*1024*1024:
		return nil, hubError(413, "upload_too_large", "ZIP archive exceeds the configured limit")
	case r.zip != nil && !bytes.HasPrefix(r.zip, []byte("PK")):
		return nil, hubError(422, "invalid_diagnostic_zip", "invalid ZIP archive")
	}
	for _, old := range d.reports {
		if r.key != "" && old.key == r.key {
			return created(object{{"report", object{{"id", old.id}}}, {"duplicate", true}}, nil)
		}
	}
	r.id = len(d.reports) + 1
	d.reports = append(d.reports, r)
	return created(object{{"report", object{{"id", r.id}, {"product", r.fields["product"]}, {"kind", r.fields["kind"]}}}, {"duplicate", false}}, nil)
}

func (m *mock) reportsJSON() object {
	d := m.desk()
	return object{
		{"tried", d.tried},
		{"reports", listOf(d.reports, func(r *hubReport) any {
			fields := object{}
			for _, k := range []string{"title", "description", "version", "product", "kind"} {
				if v, given := r.fields[k]; given {
					fields = fields.set(k, v)
				}
			}
			return object{
				{"id", r.id},
				{"fields", fields},
				{"idempotency_key", r.key},
				{"user_agent", r.agent},
				{"file_name", r.fileName},
				{"zip_base64", base64.StdEncoding.EncodeToString(r.zip)},
			}
		})},
	}
}

func (m *mock) failReports(status, count string) (any, error) {
	if !isDigits(status) || !isDigits(cmp.Or(count, "1")) {
		return nil, refusal(400, "status= takes an HTTP status and count= a number")
	}
	for range digitsValue(cmp.Or(count, "1")) {
		m.desk().fails = append(m.desk().fails, digitsValue(status))
	}
	return m.reportsJSON(), nil
}
