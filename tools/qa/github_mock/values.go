package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf16"
	"unicode/utf8"
)

type member struct {
	key   string
	value any
}

// object is a JSON object that keeps its keys in insertion order, as a Python dict does.
type object []member

func (o object) get(key string) (any, bool) {
	for _, entry := range o {
		if entry.key == key {
			return entry.value, true
		}
	}
	return nil, false
}

func (o object) has(key string) bool {
	_, present := o.get(key)
	return present
}

// text is the value under key when it is a string.
func (o object) text(key string) (string, bool) {
	value, _ := o.get(key)
	text, isText := value.(string)
	return text, isText
}

func (o object) set(key string, value any) object {
	for i := range o {
		if o[i].key == key {
			o[i].value = value
			return o
		}
	}
	return append(o, member{key, value})
}

// dumps writes v exactly as Python's json.dumps does: ", " and ": " separators, everything outside ASCII as \uXXXX.
func dumps(v any) []byte {
	return appendJSON(nil, v)
}

func appendJSON(b []byte, v any) []byte {
	switch v := v.(type) {
	case nil:
		return append(b, "null"...)
	case bool:
		return strconv.AppendBool(b, v)
	case int:
		return strconv.AppendInt(b, int64(v), 10)
	case int64:
		return strconv.AppendInt(b, v, 10)
	case json.Number:
		return append(b, v...)
	case string:
		return appendJSONString(b, v)
	case []any:
		b = append(b, '[')
		for i, item := range v {
			if i > 0 {
				b = append(b, ", "...)
			}
			b = appendJSON(b, item)
		}
		return append(b, ']')
	case object:
		b = append(b, '{')
		for i, entry := range v {
			if i > 0 {
				b = append(b, ", "...)
			}
			b = appendJSONString(b, entry.key)
			b = append(b, ": "...)
			b = appendJSON(b, entry.value)
		}
		return append(b, '}')
	}
	panic(fmt.Sprintf("no JSON form for %T", v))
}

var jsonEscapes = map[rune]string{'"': `\"`, '\\': `\\`, '\b': `\b`, '\f': `\f`, '\n': `\n`, '\r': `\r`, '\t': `\t`}

func appendJSONString(b []byte, s string) []byte {
	b = append(b, '"')
	for _, r := range s {
		short, escaped := jsonEscapes[r]
		switch {
		case escaped:
			b = append(b, short...)
		case r >= ' ' && r <= '~':
			b = append(b, byte(r))
		case r >= 0x10000:
			first, second := utf16.EncodeRune(r)
			b = fmt.Appendf(b, `\u%04x\u%04x`, first, second)
		default:
			b = fmt.Appendf(b, `\u%04x`, r)
		}
	}
	return append(b, '"')
}

// parseJSON decodes a document into nil, bool, string, json.Number, []any and object.
func parseJSON(raw []byte) (value any, valid bool) {
	if !utf8.Valid(raw) || !json.Valid(raw) {
		return nil, false
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.UseNumber()
	return readJSONValue(decoder), true
}

func readJSONValue(decoder *json.Decoder) any {
	token, _ := decoder.Token()
	switch token {
	case json.Delim('['):
		list := []any{}
		for decoder.More() {
			list = append(list, readJSONValue(decoder))
		}
		decoder.Token()
		return list
	case json.Delim('{'):
		result := object{}
		for decoder.More() {
			key, _ := decoder.Token()
			result = result.set(key.(string), readJSONValue(decoder))
		}
		decoder.Token()
		return result
	}
	return token
}

func truthy(v any) bool {
	switch v := v.(type) {
	case nil:
		return false
	case bool:
		return v
	case string:
		return v != ""
	case json.Number:
		number, _ := v.Float64()
		return number != 0
	case []any:
		return len(v) > 0
	case object:
		return len(v) > 0
	}
	return true
}

func listOf[T any](items []T, convert func(T) any) []any {
	list := make([]any, len(items))
	for i, item := range items {
		list[i] = convert(item)
	}
	return list
}

func newestFirst[T any](items []T) []T {
	reversed := make([]T, len(items))
	for i, item := range items {
		reversed[len(items)-1-i] = item
	}
	return reversed
}

// orNull is JSON null for the zero value: an empty text, an id of 0.
func orNull[T comparable](value T) any {
	var zero T
	if value == zero {
		return nil
	}
	return value
}

func isDigits(s string) bool {
	return s != "" && strings.IndexFunc(s, func(r rune) bool { return r < '0' || r > '9' }) < 0
}

// digitsValue saturates: Python's integers have no upper limit.
func digitsValue(digits string) int {
	value, err := strconv.Atoi(digits)
	if err != nil {
		return math.MaxInt
	}
	return value
}

func parseID(text string) (int, error) {
	if !isDigits(text) {
		return 0, notFound()
	}
	return digitsValue(text), nil
}

// pageOf is the slice ?per_page= (30 by default, 100 at most) and ?page= (from 1) select.
func pageOf[T any](items []T, query map[string]string) []T {
	number := func(key string, fallback int) int {
		if text := query[key]; isDigits(text) && digitsValue(text) > 0 {
			return digitsValue(text)
		}
		return fallback
	}
	perPage := min(number("per_page", 30), 100)
	page := number("page", 1)
	if page-1 > len(items)/perPage {
		return items[:0]
	}
	start := min((page-1)*perPage, len(items))
	return items[start:min(start+perPage, len(items))]
}

func strip(s string) string {
	return strings.TrimFunc(s, func(r rune) bool { return unicode.IsSpace(r) || r >= 0x1c && r <= 0x1f })
}

// latin1 reads raw bytes the way Python's http.server reads the request line and the headers.
func latin1(raw string) string {
	decoded := make([]rune, len(raw))
	for i := range len(raw) {
		decoded[i] = rune(raw[i])
	}
	return string(decoded)
}

// replaceInvalid is Python's decode("utf-8", "replace").
func replaceInvalid(s string) string {
	return string([]rune(s))
}

// unquote is urllib's unquote: an escape that is not one stays as it is.
func unquote(s string) string {
	decoded := make([]byte, 0, len(s))
	for i := 0; i < len(s); i++ {
		if s[i] == '%' && i+3 <= len(s) {
			if value, err := hex.DecodeString(s[i+1 : i+3]); err == nil {
				decoded = append(decoded, value[0])
				i += 2
				continue
			}
		}
		decoded = append(decoded, s[i])
	}
	return replaceInvalid(string(decoded))
}

// parseQuery is parse_qs with the last value of every key kept.
func parseQuery(text string, keepBlank bool) map[string]string {
	values := map[string]string{}
	for pair := range strings.SplitSeq(text, "&") {
		key, value, _ := strings.Cut(pair, "=")
		if pair == "" || value == "" && !keepBlank {
			continue
		}
		values[unquote(strings.ReplaceAll(key, "+", " "))] = unquote(strings.ReplaceAll(value, "+", " "))
	}
	return values
}

const quoteSafe = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.-~/"

// quote is urllib's quote with its default safe "/".
func quote(s string) string {
	var quoted strings.Builder
	for i := 0; i < len(s); i++ {
		if strings.IndexByte(quoteSafe, s[i]) >= 0 {
			quoted.WriteByte(s[i])
		} else {
			fmt.Fprintf(&quoted, "%%%02X", s[i])
		}
	}
	return quoted.String()
}

// pyRepr is Python's repr() for the values a problem message quotes; other JSON values keep their JSON text.
func pyRepr(v any) string {
	switch v := v.(type) {
	case nil:
		return "None"
	case bool:
		if v {
			return "True"
		}
		return "False"
	case string:
		mark := '\''
		if strings.Contains(v, "'") && !strings.Contains(v, `"`) {
			mark = '"'
		}
		var repr strings.Builder
		repr.WriteRune(mark)
		for _, r := range v {
			switch {
			case r == mark || r == '\\':
				repr.WriteRune('\\')
				repr.WriteRune(r)
			case r == '\n':
				repr.WriteString(`\n`)
			case r == '\r':
				repr.WriteString(`\r`)
			case r == '\t':
				repr.WriteString(`\t`)
			case unicode.IsPrint(r):
				repr.WriteRune(r)
			case r < 0x100:
				fmt.Fprintf(&repr, `\x%02x`, r)
			case r < 0x10000:
				fmt.Fprintf(&repr, `\u%04x`, r)
			default:
				fmt.Fprintf(&repr, `\U%08x`, r)
			}
		}
		repr.WriteRune(mark)
		return repr.String()
	}
	return string(dumps(v))
}
