package main

// The release assets as the editor's updater reads them: anonymously, by range, from the CDN
// path a download redirects to. The harness breaks and counts those reads:
//
//	POST /__asset/drop?after=N[&name=X][&times=K][&hold=MS]
//	                    the next K (default 1) asset answers longer than N bytes (of the asset
//	                    named X, or any) send N body bytes, hold the connection MS
//	                    milliseconds (default 0) and close it
//	POST /__asset/noranges?on=1|0
//	                    a ranged request is answered 200 with the whole asset, as a server that
//	                    ignores ranges does
//	GET  /__asset/stats {"assets": {"<name>": {"requests": n, "bytes": sent, "reads": [{"range":
//	                    the Range header, "first", "last", "status", "sent"}]}}}, oldest first
//	POST /__asset/reset the counts, a drop rule and noranges forgotten

import (
	"fmt"
	"slices"
	"strconv"
	"time"
)

type assetDrop struct {
	after, times int
	name         string
	hold         time.Duration
}

type assetRead struct {
	header              string
	first, last, status int
	sent                int
}

type assetStat struct {
	reads []*assetRead
}

// anonymousRead: GET /repos/{repo}/releases/latest without an Authorization header, which a
// public repository answers on GitHub too. Any header at all is checked as before.
func (x *exchange) anonymousRead() bool {
	seg := x.segments
	_, authorized := x.r.Header["Authorization"]
	return x.r.Method == "GET" && !authorized && x.r.Header.Get("User-Agent") != "" &&
		match(seg, "repos", "*", "*", "releases", "latest") && x.mock.isRepo(seg[1], seg[2]) && !x.mock.cfg.private
}

// assetServed counts an answer and arms the drop rule on it. Callers hold mu; the bytes sent are
// counted once the answer went out.
func (m *mock) assetServed(name, header string, first, last int, a *answer) {
	stat := m.assetStats[name]
	if stat == nil {
		stat = &assetStat{}
		m.assetStats[name] = stat
	}
	read := &assetRead{header: header, first: first, last: last, status: a.status}
	stat.reads = append(stat.reads, read)
	if rule := m.assetDrop; rule != nil && rule.times > 0 && (rule.name == "" || rule.name == name) && rule.after < len(a.body) {
		rule.times--
		a.cutting, a.cut, a.hold = true, rule.after, rule.hold
		if rule.times == 0 {
			m.assetDrop = nil
		}
	}
	a.sent = func(n int) {
		m.mu.Lock()
		defer m.mu.Unlock()
		read.sent = n
	}
}

func (m *mock) assetStatsJSON() object {
	names := make([]string, 0, len(m.assetStats))
	for name := range m.assetStats {
		names = append(names, name)
	}
	slices.Sort(names)
	assets := object{}
	for _, name := range names {
		stat := m.assetStats[name]
		total := 0
		reads := []any{}
		for _, r := range stat.reads {
			total += r.sent
			reads = append(reads, object{{"range", r.header}, {"first", r.first}, {"last", r.last}, {"status", r.status}, {"sent", r.sent}})
		}
		assets = append(assets, member{name, object{{"requests", len(stat.reads)}, {"bytes", total}, {"reads", reads}}})
	}
	return object{{"assets", assets}, {"noranges", m.noRanges}, {"drop", m.assetDrop != nil}}
}

func (m *mock) assetHarness(what, method string, query map[string]string) (*answer, error) {
	number := func(key string, fallback int) (int, error) {
		text, given := query[key]
		if !given || text == "" {
			return fallback, nil
		}
		if !isDigits(text) {
			return 0, refusal(400, fmt.Sprintf("%s= takes a count", key))
		}
		return strconv.Atoi(text)
	}
	switch {
	case method == "GET" && what == "stats":
		return ok(m.assetStatsJSON(), nil)
	case method == "POST" && what == "reset":
		m.assetStats = map[string]*assetStat{}
		m.assetDrop = nil
		m.noRanges = false
		return ok(m.assetStatsJSON(), nil)
	case method == "POST" && what == "noranges":
		m.noRanges = query["on"] != "0"
		return ok(m.assetStatsJSON(), nil)
	case method == "POST" && what == "drop":
		after, err := number("after", -1)
		if err != nil || after < 0 {
			return nil, refusal(400, "after= takes the bytes to send before the cut")
		}
		times, err := number("times", 1)
		if err != nil || times < 1 {
			return nil, refusal(400, "times= takes a count from 1")
		}
		hold, err := number("hold", 0)
		if err != nil {
			return nil, err
		}
		m.assetDrop = &assetDrop{after: after, times: times, name: query["name"], hold: time.Duration(hold) * time.Millisecond}
		return ok(m.assetStatsJSON(), nil)
	}
	return nil, notFound()
}
