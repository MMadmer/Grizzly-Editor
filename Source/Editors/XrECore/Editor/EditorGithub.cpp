#include "stdafx.h"
#include "EditorGithub.h"
#include "Nq/NqUtil.h"
#include <winhttp.h>
#include <wincred.h>
#include <bcrypt.h>
#include <string>

#pragma comment(lib, "bcrypt.lib")

#ifndef WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3
#define WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3 0x00002000
#endif

namespace
{
// The client id of the XFined Editor OAuth app on GitHub (Settings > Developer
// settings > OAuth Apps, "Enable Device Flow" ticked). It is public by design -
// the device flow has no client secret. Empty = the sign-in button is not
// offered and the account comes from a pasted token, the environment or gh.
const char* kClientId = "";

// The Mod Browser catalog of Dead Air: Refined and where its public files are served; the game
// carries the same mirrors (xms_catalog.cpp, kMirrors).
const char* kCatalogRepo = "MMadmer/darf-mods";
const char* kCatalogMirrors[] = {
	"https://mmadmer.github.io/darf-mods/",
	"https://cdn.jsdelivr.net/gh/MMadmer/darf-mods@main/public/",
	"https://raw.githubusercontent.com/MMadmer/darf-mods/main/public/",
};
// The catalog's workflow comments its verdicts as this account. A person cannot take a login with
// brackets, so a comment by it is the workflow's own.
const char* kCatalogBot = "github-actions[bot]";

const u32 kMaxResponse		= 32u << 20;
const u32 kUploadChunk		= 256u << 10;
const u32 kUploadAttempts	= 3;
// A release under a tag that was never used is served at once; one under a tag that is used
// again is not - see the warning at the end of Publish. Long enough to tell the two apart.
const u32 kServeAttempts	= 15;

//------------------------------------------------------------------------------
// QA
//------------------------------------------------------------------------------
// -qa_github points the client at a mock through XFINED_GITHUB_API / _WEB. In
// that mode the account comes from XFINED_GITHUB_TOKEN or the QA credential
// alone: a real token never travels to a host that is not GitHub.
bool QaMode()
{
	static const bool qa = 0 != strstr(::GetCommandLineA(), "-qa_github");
	return qa;
}

xr_string EnvText(LPCSTR name)
{
	char buf[4096];
	const DWORD n = ::GetEnvironmentVariableA(name, buf, sizeof(buf));
	return (n && n < sizeof(buf)) ? xr_string(buf) : xr_string();
}

xr_string Base(LPCSTR variable, LPCSTR real)
{
	xr_string v = QaMode() ? EnvText(variable) : xr_string();
	if (v.empty()) v = real;
	while (!v.empty() && v[v.size() - 1] == '/') v.erase(v.size() - 1);
	return v;
}

xr_string ApiBase()	{ return Base("XFINED_GITHUB_API", "https://api.github.com"); }
xr_string WebBase()	{ return Base("XFINED_GITHUB_WEB", "https://github.com"); }
xr_string VtBase()	{ return Base("XFINED_VT_API", "https://www.virustotal.com/api/v3"); }

xr_string ClientId()
{
	const xr_string qa = QaMode() ? EnvText("XFINED_GITHUB_CLIENT_ID") : xr_string();
	return qa.empty() ? xr_string(kClientId) : qa;
}

// scheme://host[:port]
xr_string Origin(const xr_string& url)
{
	const size_t scheme = url.find("://");
	if (scheme == xr_string::npos) return xr_string();
	const size_t slash = url.find('/', scheme + 3);
	return url.substr(0, slash);
}

// The token goes wherever the upload URL points, and that URL comes out of a
// response body.
bool UploadUrlTrusted(const xr_string& url)
{
	const xr_string origin = Origin(url);
	const bool mocked = QaMode() && !EnvText("XFINED_GITHUB_API").empty();
	return 0 == _stricmp(origin.c_str(), mocked ? Origin(ApiBase()).c_str() : "https://uploads.github.com");
}

//------------------------------------------------------------------------------
// text
//------------------------------------------------------------------------------
std::wstring Widen(const xr_string& s)
{
	if (s.empty()) return std::wstring();
	const int n = ::MultiByteToWideChar(CP_UTF8, 0, s.c_str(), (int)s.size(), 0, 0);
	std::wstring w((size_t)n, L'\0');
	::MultiByteToWideChar(CP_UTF8, 0, s.c_str(), (int)s.size(), &w[0], n);
	return w;
}

xr_string UrlEncode(const xr_string& s)
{
	xr_string out;
	for (size_t i = 0; i < s.size(); ++i)
	{
		const unsigned char c = (unsigned char)s[i];
		if ((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '-' || c == '.' || c == '_' || c == '~')
			out += (char)c;
		else
			out += NqUtil::Format("%%%02X", (u32)c);
	}
	return out;
}

void JsonEscape(xr_string& out, const xr_string& s)
{
	for (size_t i = 0; i < s.size(); ++i)
	{
		const unsigned char c = (unsigned char)s[i];
		switch (c)
		{
		case '"':	out += "\\\""; break;
		case '\\':	out += "\\\\"; break;
		case '\n':	out += "\\n"; break;
		case '\r':	out += "\\r"; break;
		case '\t':	out += "\\t"; break;
		default:
			if (c < 0x20)	out += NqUtil::Format("\\u%04x", (u32)c);
			else			out += (char)c;
		}
	}
}

//------------------------------------------------------------------------------
// JSON, as much of it as GitHub's answers need
//------------------------------------------------------------------------------
struct SJson
{
	enum EType { jNull, jBool, jNumber, jString, jArray, jObject };
	EType					type;
	bool					flag;
	u64						integer;	// a plain non-negative integer literal; ids are one
	xr_string				text;
	xr_vector<SJson>		items;
	xr_vector<xr_string>	keys;		// object: keys[i] names items[i]

	SJson() : type(jNull), flag(false), integer(0) {}

	const SJson* Find(LPCSTR key) const
	{
		if (type != jObject) return 0;
		for (u32 i = 0; i < keys.size(); ++i)
			if (keys[i] == key) return &items[i];
		return 0;
	}
	LPCSTR Str(LPCSTR key) const
	{
		const SJson* v = Find(key);
		return (v && v->type == jString) ? v->text.c_str() : "";
	}
	bool Bool(LPCSTR key) const
	{
		const SJson* v = Find(key);
		return v && v->type == jBool && v->flag;
	}
	u64 U64(LPCSTR key) const
	{
		const SJson* v = Find(key);
		return (v && v->type == jNumber) ? v->integer : 0;
	}
};

class CJsonReader
{
public:
	explicit CJsonReader(const xr_string& s) : m_P(s.c_str()), m_End(s.c_str() + s.size()), m_Depth(0) {}

	bool Parse(SJson& out)
	{
		Skip();
		if (!Value(out)) return false;
		Skip();
		return m_P == m_End;
	}

private:
	const char*	m_P;
	const char*	m_End;
	u32			m_Depth;

	void Skip()
	{
		while (m_P < m_End && (*m_P == ' ' || *m_P == '\t' || *m_P == '\r' || *m_P == '\n')) ++m_P;
	}

	bool Word(LPCSTR word)
	{
		const size_t n = xr_strlen(word);
		if ((size_t)(m_End - m_P) < n || 0 != strncmp(m_P, word, n)) return false;
		m_P += n;
		return true;
	}

	static void AppendUtf8(xr_string& out, u32 cp)
	{
		if (cp < 0x80)			out += (char)cp;
		else if (cp < 0x800)	{ out += (char)(0xC0 | (cp >> 6)); out += (char)(0x80 | (cp & 0x3F)); }
		else if (cp < 0x10000)	{ out += (char)(0xE0 | (cp >> 12)); out += (char)(0x80 | ((cp >> 6) & 0x3F)); out += (char)(0x80 | (cp & 0x3F)); }
		else					{ out += (char)(0xF0 | (cp >> 18)); out += (char)(0x80 | ((cp >> 12) & 0x3F));
								  out += (char)(0x80 | ((cp >> 6) & 0x3F)); out += (char)(0x80 | (cp & 0x3F)); }
	}

	bool Hex4(u32& value)
	{
		if (m_End - m_P < 4) return false;
		value = 0;
		for (int i = 0; i < 4; ++i, ++m_P)
		{
			const char c = *m_P;
			value <<= 4;
			if (c >= '0' && c <= '9')		value |= (u32)(c - '0');
			else if (c >= 'a' && c <= 'f')	value |= (u32)(c - 'a' + 10);
			else if (c >= 'A' && c <= 'F')	value |= (u32)(c - 'A' + 10);
			else return false;
		}
		return true;
	}

	bool String(xr_string& out)
	{
		if (m_P >= m_End || *m_P != '"') return false;
		++m_P;
		while (m_P < m_End && *m_P != '"')
		{
			if (*m_P != '\\') { out += *m_P++; continue; }
			if (++m_P >= m_End) return false;
			const char e = *m_P++;
			switch (e)
			{
			case 'b':	out += '\b'; break;
			case 'f':	out += '\f'; break;
			case 'n':	out += '\n'; break;
			case 'r':	out += '\r'; break;
			case 't':	out += '\t'; break;
			case 'u':
			{
				u32 cp = 0, low = 0;
				if (!Hex4(cp)) return false;
				// a surrogate pair is one character
				if (cp >= 0xD800 && cp <= 0xDBFF && m_End - m_P >= 6 && m_P[0] == '\\' && m_P[1] == 'u')
				{
					const char* keep = m_P;
					m_P += 2;
					if (Hex4(low) && low >= 0xDC00 && low <= 0xDFFF)
						cp = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
					else
						m_P = keep;
				}
				AppendUtf8(out, cp);
				break;
			}
			default:	out += e;	// \" \\ \/
			}
		}
		if (m_P >= m_End) return false;
		++m_P;
		return true;
	}

	bool Number(SJson& out)
	{
		const char* start = m_P;
		while (m_P < m_End && (strchr("+-.eE", *m_P) || (*m_P >= '0' && *m_P <= '9'))) ++m_P;
		if (m_P == start) return false;
		out.type = SJson::jNumber;
		bool plain = true;
		u64 value = 0;
		for (const char* c = start; c < m_P; ++c)
		{
			if (*c < '0' || *c > '9') { plain = false; break; }
			value = value * 10 + (u64)(*c - '0');
		}
		out.integer = plain ? value : 0;
		return true;
	}

	bool Value(SJson& out)
	{
		if (m_P >= m_End || ++m_Depth > 64) return false;
		bool ok = false;
		const char c = *m_P;
		if (c == '{')
		{
			++m_P;
			out.type = SJson::jObject;
			Skip();
			if (m_P < m_End && *m_P == '}') { ++m_P; ok = true; }
			else for (;;)
			{
				xr_string key;
				Skip();
				if (!String(key)) break;
				Skip();
				if (m_P >= m_End || *m_P++ != ':') break;
				Skip();
				out.keys.push_back(key);
				out.items.push_back(SJson());
				if (!Value(out.items.back())) break;
				Skip();
				if (m_P >= m_End) break;
				if (*m_P == ',') { ++m_P; continue; }
				if (*m_P == '}') { ++m_P; ok = true; }
				break;
			}
		}
		else if (c == '[')
		{
			++m_P;
			out.type = SJson::jArray;
			Skip();
			if (m_P < m_End && *m_P == ']') { ++m_P; ok = true; }
			else for (;;)
			{
				Skip();
				out.items.push_back(SJson());
				if (!Value(out.items.back())) break;
				Skip();
				if (m_P >= m_End) break;
				if (*m_P == ',') { ++m_P; continue; }
				if (*m_P == ']') { ++m_P; ok = true; }
				break;
			}
		}
		else if (c == '"')		{ out.type = SJson::jString; ok = String(out.text); }
		else if (Word("true"))	{ out.type = SJson::jBool; out.flag = true; ok = true; }
		else if (Word("false"))	{ out.type = SJson::jBool; ok = true; }
		else if (Word("null"))	{ ok = true; }
		else					ok = Number(out);
		--m_Depth;
		return ok;
	}
};

bool ParseJson(const xr_string& text, SJson& out)
{
	out = SJson();
	return CJsonReader(text).Parse(out);
}

//------------------------------------------------------------------------------
// HTTP
//------------------------------------------------------------------------------
struct SHttpResponse
{
	u32			status;		// 0 = no answer at all
	xr_string	body;
	xr_string	error;		// why there was none

	SHttpResponse() : status(0) {}
};

struct SHttpRequest
{
	LPCSTR					method;
	xr_string				url;
	xr_vector<xr_string>	headers;
	xr_string				body;
	xr_string				body_file;		// streamed instead of `body`
	xr_string				body_prefix;	// sent around `body_file`: a multipart form's
	xr_string				body_suffix;	// part headers and closing boundary
	bool					redirects;		// follow them; never with a token on board

	SHttpRequest() : method("GET"), redirects(false) {}
};

typedef std::function<void(u64 sent, u64 total)> TUploadProgress;

xr_string NetworkError(DWORD code)
{
	switch (code)
	{
	case ERROR_WINHTTP_NAME_NOT_RESOLVED:	return "no connection: the host name was not resolved";
	case ERROR_WINHTTP_CANNOT_CONNECT:		return "no connection to the server";
	case ERROR_WINHTTP_TIMEOUT:				return "the connection timed out";
	case ERROR_WINHTTP_CONNECTION_ERROR:	return "the connection was lost";
	case ERROR_WINHTTP_SECURE_FAILURE:		return "the secure connection failed";
	case ERROR_WINHTTP_OPERATION_CANCELLED:	return "cancelled";
	}
	return NqUtil::Format("network error %u", (u32)code);
}

class CHttp
{
public:
	CHttp() : m_Session(0)
	{
		m_Session = ::WinHttpOpen(L"XFined-Editor", WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
			WINHTTP_NO_PROXY_NAME, WINHTTP_NO_PROXY_BYPASS, 0);
		if (!m_Session)
			m_Session = ::WinHttpOpen(L"XFined-Editor", WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
				WINHTTP_NO_PROXY_NAME, WINHTTP_NO_PROXY_BYPASS, 0);
		if (!m_Session) return;
		::WinHttpSetTimeouts(m_Session, 15000, 15000, 60000, 60000);
		DWORD protocols = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3;
		if (!::WinHttpSetOption(m_Session, WINHTTP_OPTION_SECURE_PROTOCOLS, &protocols, sizeof(protocols)))
		{
			protocols = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2;
			::WinHttpSetOption(m_Session, WINHTTP_OPTION_SECURE_PROTOCOLS, &protocols, sizeof(protocols));
		}
	}
	~CHttp()		{ if (m_Session) ::WinHttpCloseHandle(m_Session); }
	bool Ready() const	{ return 0 != m_Session; }

	SHttpResponse Send(const SHttpRequest& rq, EditorGithub::CCancel& cancel, const TUploadProgress& progress)
	{
		SHttpResponse out;
		if (cancel.Cancelled())	{ out.error = "cancelled"; return out; }
		if (!m_Session)			{ out.error = "WinHTTP is not available"; return out; }

		const std::wstring url = Widen(rq.url);
		wchar_t host[256] = {}, path[4096] = {}, extra[4096] = {};
		URL_COMPONENTS uc = {};
		uc.dwStructSize		= sizeof(uc);
		uc.lpszHostName		= host;		uc.dwHostNameLength		= _countof(host);
		uc.lpszUrlPath		= path;		uc.dwUrlPathLength		= _countof(path);
		uc.lpszExtraInfo	= extra;	uc.dwExtraInfoLength	= _countof(extra);
		if (!::WinHttpCrackUrl(url.c_str(), 0, 0, &uc)) { out.error = "invalid address"; return out; }
		const bool secure = INTERNET_SCHEME_HTTPS == uc.nScheme;
		const std::wstring object = std::wstring(path) + extra;

		HINTERNET connect = ::WinHttpConnect(m_Session, host, uc.nPort, 0);
		if (!connect) { out.error = NetworkError(::GetLastError()); return out; }
		HINTERNET request = ::WinHttpOpenRequest(connect, Widen(rq.method).c_str(), object.c_str(), 0,
			WINHTTP_NO_REFERER, WINHTTP_DEFAULT_ACCEPT_TYPES, secure ? WINHTTP_FLAG_SECURE : 0);
		if (!request)
		{
			out.error = NetworkError(::GetLastError());
			::WinHttpCloseHandle(connect);
			return out;
		}
		cancel.Attach(request);

		HANDLE file = INVALID_HANDLE_VALUE;
		if (Exchange(rq, request, cancel, progress, file, out))
			out.error.clear();
		else if (cancel.Cancelled())
			out.error = "cancelled";
		if (!out.error.empty()) { out.status = 0; out.body.clear(); }

		if (INVALID_HANDLE_VALUE != file) ::CloseHandle(file);
		if (cancel.Detach()) ::WinHttpCloseHandle(request);
		::WinHttpCloseHandle(connect);
		return out;
	}

private:
	HINTERNET m_Session;

	static bool Fail(SHttpResponse& out, LPCSTR text = 0)
	{
		out.error = text ? xr_string(text) : NetworkError(::GetLastError());
		return false;
	}

	static bool WriteAll(HINTERNET request, const xr_string& data)
	{
		for (size_t done = 0; done < data.size();)
		{
			DWORD written = 0;
			if (!::WinHttpWriteData(request, data.data() + done, (DWORD)(data.size() - done), &written) || !written)
				return false;
			done += written;
		}
		return true;
	}

	bool Exchange(const SHttpRequest& rq, HINTERNET request, EditorGithub::CCancel& cancel,
				  const TUploadProgress& progress, HANDLE& file, SHttpResponse& out)
	{
		if (!rq.redirects)
		{
			DWORD policy = WINHTTP_OPTION_REDIRECT_POLICY_NEVER;
			::WinHttpSetOption(request, WINHTTP_OPTION_REDIRECT_POLICY, &policy, sizeof(policy));
		}
		for (u32 i = 0; i < rq.headers.size(); ++i)
			if (!::WinHttpAddRequestHeaders(request, Widen(rq.headers[i]).c_str(), (DWORD)-1L,
					WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE))
				return Fail(out);

		if (rq.body_file.empty())
		{
			if (!::WinHttpSendRequest(request, WINHTTP_NO_ADDITIONAL_HEADERS, 0,
					rq.body.empty() ? WINHTTP_NO_REQUEST_DATA : (LPVOID)rq.body.c_str(),
					(DWORD)rq.body.size(), (DWORD)rq.body.size(), 0))
				return Fail(out);
		}
		else
		{
			// GitHub answers an upload only after it has taken the whole asset in -
			// for a package of a gigabyte that is well past the default minute
			::WinHttpSetTimeouts(request, 15000, 15000, 120000, 600000);
			file = ::CreateFileA(rq.body_file.c_str(), GENERIC_READ, FILE_SHARE_READ, 0, OPEN_EXISTING,
				FILE_FLAG_SEQUENTIAL_SCAN, 0);
			LARGE_INTEGER size = {};
			if (INVALID_HANDLE_VALUE == file || !::GetFileSizeEx(file, &size))
				return Fail(out, "cannot read the file to upload");
			// the length travels as a DWORD; a release asset is under 2 GiB anyway
			const u64 wrapping = rq.body_prefix.size() + rq.body_suffix.size();
			if ((u64)size.QuadPart + wrapping > 0xFFFFFFFFull)
				return Fail(out, "the file is too large to upload");
			const u64 total = (u64)size.QuadPart;
			if (!::WinHttpSendRequest(request, WINHTTP_NO_ADDITIONAL_HEADERS, 0, WINHTTP_NO_REQUEST_DATA, 0, (DWORD)(total + wrapping), 0) ||
				!WriteAll(request, rq.body_prefix))
				return Fail(out);

			xr_vector<u8> chunk(kUploadChunk);
			u64 sent = 0;
			while (sent < total)
			{
				if (cancel.Cancelled()) return Fail(out, "cancelled");
				DWORD got = 0;
				if (!::ReadFile(file, chunk.data(), (DWORD)chunk.size(), &got, 0) || !got)
					return Fail(out, "cannot read the file to upload");
				DWORD done = 0;
				while (done < got)
				{
					DWORD written = 0;
					if (!::WinHttpWriteData(request, chunk.data() + done, got - done, &written) || !written)
						return Fail(out);
					done += written;
				}
				sent += got;
				if (progress) progress(sent, total);
			}
			if (!WriteAll(request, rq.body_suffix)) return Fail(out);
		}

		if (!::WinHttpReceiveResponse(request, 0)) return Fail(out);
		DWORD status = 0, status_size = sizeof(status);
		if (!::WinHttpQueryHeaders(request, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
				WINHTTP_HEADER_NAME_BY_INDEX, &status, &status_size, WINHTTP_NO_HEADER_INDEX))
			return Fail(out);
		out.status = status;

		xr_vector<char> buf(64u << 10);
		for (;;)
		{
			DWORD got = 0;
			if (!::WinHttpReadData(request, buf.data(), (DWORD)buf.size(), &got)) return Fail(out);
			if (!got) break;
			if (out.body.size() + got > kMaxResponse) return Fail(out, "the answer is too large");
			out.body.append(buf.data(), got);
		}
		return true;
	}
};

// what GitHub said, in its own words where it has any
xr_string Problem(const SHttpResponse& r)
{
	if (!r.status) return r.error;
	xr_string text;
	SJson j;
	if (ParseJson(r.body, j))
	{
		text = j.Str("message");
		const SJson* errors = j.Find("errors");
		if (errors && errors->type == SJson::jArray && !errors->items.empty())
		{
			// an entry is an object with code and field, or the sentence itself
			const SJson& first = errors->items[0];
			xr_string detail = SJson::jString == first.type ? first.text : xr_string(first.Str("message"));
			if (detail.empty() && first.Str("code")[0])
				detail = xr_string(first.Str("field")) + " " + first.Str("code");
			if (!detail.empty()) text += " (" + detail + ")";
		}
	}
	return text.empty() ? NqUtil::Format("GitHub answered %u", r.status)
						: NqUtil::Format("GitHub answered %u: %s", r.status, text.c_str());
}

class CApi
{
public:
	CApi(CHttp& http, EditorGithub::CCancel& cancel, const xr_string& token)
		: m_Http(http), m_Cancel(cancel), m_Token(token) {}
	~CApi() { EditorGithub::WipeString(m_Token); }

	SHttpResponse Get(const xr_string& url)									{ return Call("GET", url, 0, 0, TUploadProgress()); }
	SHttpResponse Delete(const xr_string& url)								{ return Call("DELETE", url, 0, 0, TUploadProgress()); }
	SHttpResponse Send(LPCSTR method, const xr_string& url, const xr_string& json)	{ return Call(method, url, &json, 0, TUploadProgress()); }
	SHttpResponse Upload(const xr_string& url, const xr_string& file, const TUploadProgress& progress)
																			{ return Call("POST", url, 0, file.c_str(), progress); }

private:
	CHttp&					m_Http;
	EditorGithub::CCancel&	m_Cancel;
	xr_string				m_Token;

	SHttpResponse Call(LPCSTR method, const xr_string& url, const xr_string* json, LPCSTR file, const TUploadProgress& progress)
	{
		SHttpRequest rq;
		rq.method = method;
		rq.url = url;
		rq.headers.push_back("Accept: application/vnd.github+json");
		rq.headers.push_back("X-GitHub-Api-Version: 2022-11-28");
		rq.headers.push_back("Authorization: Bearer " + m_Token);
		if (json)	{ rq.headers.push_back("Content-Type: application/json"); rq.body = *json; }
		if (file)	{ rq.headers.push_back("Content-Type: application/octet-stream"); rq.body_file = file; }
		SHttpResponse r = m_Http.Send(rq, m_Cancel, progress);
		EditorGithub::WipeString(rq.headers[2]);
		return r;
	}
};

bool SameName(LPCSTR a, LPCSTR b) { return 0 == _stricmp(a, b); }

bool EndsWith(const xr_string& s, LPCSTR tail)
{
	const size_t n = xr_strlen(tail);
	return s.size() >= n && 0 == _stricmp(s.c_str() + s.size() - n, tail);
}

bool ReadWholeFile(LPCSTR path, xr_string& out)
{
	out.clear();
	HANDLE h = ::CreateFileA(path, GENERIC_READ, FILE_SHARE_READ, 0, OPEN_EXISTING, 0, 0);
	if (INVALID_HANDLE_VALUE == h) return false;
	char buf[8192];
	DWORD got = 0;
	while (::ReadFile(h, buf, sizeof(buf), &got, 0) && got) out.append(buf, got);
	::CloseHandle(h);
	return true;
}

// interruptible: a cancelled job must not sit out a pause
void Pause(EditorGithub::CCancel& cancel, u32 ms)
{
	for (u32 waited = 0; waited < ms && !cancel.Cancelled(); waited += 100) ::Sleep(100);
}

//------------------------------------------------------------------------------
// account
//------------------------------------------------------------------------------
const wchar_t* CredentialTarget()
{
	return QaMode() ? L"XFinedEditor:GitHub:qa" : L"XFinedEditor:GitHub";
}

bool ReadSavedToken(xr_string& token)
{
	PCREDENTIALW cred = 0;
	if (!::CredReadW(CredentialTarget(), CRED_TYPE_GENERIC, 0, &cred)) return false;
	token.assign((const char*)cred->CredentialBlob, cred->CredentialBlobSize);
	SecureZeroMemory(cred->CredentialBlob, cred->CredentialBlobSize);
	::CredFree(cred);
	return EditorGithub::PlausibleToken(token.c_str());
}

// `gh auth token` prints the token the GitHub CLI is signed in with
bool ReadGithubCliToken(xr_string& token)
{
	wchar_t exe[MAX_PATH];
	if (!::SearchPathW(0, L"gh.exe", 0, _countof(exe), exe, 0)) return false;

	SECURITY_ATTRIBUTES inherit = { sizeof(inherit), 0, TRUE };
	HANDLE read_end = 0, write_end = 0;
	if (!::CreatePipe(&read_end, &write_end, &inherit, 0)) return false;
	::SetHandleInformation(read_end, HANDLE_FLAG_INHERIT, 0);
	HANDLE nul = ::CreateFileW(L"NUL", GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, &inherit, OPEN_EXISTING, 0, 0);

	// An explicit handle list: inheriting everything would hand gh the MCP listener.
	SIZE_T list_size = 0;
	::InitializeProcThreadAttributeList(0, 1, 0, &list_size);
	xr_vector<u8> list_memory(list_size);
	LPPROC_THREAD_ATTRIBUTE_LIST list = (LPPROC_THREAD_ATTRIBUTE_LIST)list_memory.data();
	HANDLE handles[2] = { write_end, nul };
	bool started = false;
	PROCESS_INFORMATION pi = {};
	if (INVALID_HANDLE_VALUE != nul && ::InitializeProcThreadAttributeList(list, 1, 0, &list_size))
	{
		if (::UpdateProcThreadAttribute(list, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, handles, sizeof(handles), 0, 0))
		{
			STARTUPINFOEXW si = {};
			si.StartupInfo.cb			= sizeof(si);
			si.StartupInfo.dwFlags		= STARTF_USESTDHANDLES;
			si.StartupInfo.hStdInput	= nul;
			si.StartupInfo.hStdOutput	= write_end;
			si.StartupInfo.hStdError	= nul;
			si.lpAttributeList			= list;
			std::wstring command = L"\"" + std::wstring(exe) + L"\" auth token --hostname github.com";
			started = 0 != ::CreateProcessW(exe, &command[0], 0, 0, TRUE, CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT,
				0, 0, &si.StartupInfo, &pi);
		}
		::DeleteProcThreadAttributeList(list);
	}
	::CloseHandle(write_end);
	if (INVALID_HANDLE_VALUE != nul) ::CloseHandle(nul);

	bool ok = false;
	if (started)
	{
		// one short line: it fits the pipe, so gh never blocks on us
		DWORD exit_code = 1;
		if (WAIT_OBJECT_0 == ::WaitForSingleObject(pi.hProcess, 5000))
			::GetExitCodeProcess(pi.hProcess, &exit_code);
		else
			::TerminateProcess(pi.hProcess, 1);
		char buf[512] = {};
		DWORD available = 0, got = 0;
		if (0 == exit_code && ::PeekNamedPipe(read_end, 0, 0, 0, &available, 0) && available &&
			::ReadFile(read_end, buf, std::min<DWORD>(available, sizeof(buf) - 1), &got, 0))
		{
			token.assign(buf, got);
			while (!token.empty() && (token.back() == '\n' || token.back() == '\r' || token.back() == ' ')) token.pop_back();
			ok = EditorGithub::PlausibleToken(token.c_str());
		}
		SecureZeroMemory(buf, sizeof(buf));
		::CloseHandle(pi.hThread);
		::CloseHandle(pi.hProcess);
	}
	::CloseHandle(read_end);
	if (!ok) EditorGithub::WipeString(token);
	return ok;
}

SHttpResponse PostForm(CHttp& http, EditorGithub::CCancel& cancel, const xr_string& url, const xr_string& form)
{
	SHttpRequest rq;
	rq.method = "POST";
	rq.url = url;
	rq.headers.push_back("Accept: application/json");
	rq.headers.push_back("Content-Type: application/x-www-form-urlencoded");
	rq.body = form;
	return http.Send(rq, cancel, TUploadProgress());
}
} // namespace

bool EditorGithub::QaRun() { return QaMode(); }

//------------------------------------------------------------------------------
// CCancel
//------------------------------------------------------------------------------
void EditorGithub::CCancel::Cancel()
{
	m_Flag = true;
	std::lock_guard<std::mutex> guard(m_Lock);
	if (m_Request)
	{
		::WinHttpCloseHandle((HINTERNET)m_Request);
		m_Request = 0;
	}
}

void EditorGithub::CCancel::Attach(void* request)
{
	std::lock_guard<std::mutex> guard(m_Lock);
	m_Request = request;
}

bool EditorGithub::CCancel::Detach()
{
	std::lock_guard<std::mutex> guard(m_Lock);
	const bool mine = 0 != m_Request;
	m_Request = 0;
	return mine;
}

//------------------------------------------------------------------------------
// account
//------------------------------------------------------------------------------
EditorGithub::SAccount::~SAccount() { WipeString(token); }

void EditorGithub::WipeString(xr_string& s)
{
	if (!s.empty()) SecureZeroMemory(&s[0], s.size());
	s.clear();
}

bool EditorGithub::PlausibleToken(LPCSTR token)
{
	const size_t n = token ? xr_strlen(token) : 0;
	if (n < 20 || n > 255) return false;
	for (size_t i = 0; i < n; ++i)
	{
		const char c = token[i];
		if (!((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_' || c == '-'))
			return false;
	}
	return true;
}

LPCSTR EditorGithub::SourceText(EAccountSource source)
{
	switch (source)
	{
	case asEnvironment:	return "environment variable";
	case asSaved:		return "token saved by the editor";
	case asGithubCli:	return "GitHub CLI (gh)";
	}
	return "not signed in";
}

void EditorGithub::ResolveAccount(SAccount& out)
{
	out.source = asNone;
	WipeString(out.token);
	out.detail.clear();

	static const char* variables[] = { "XFINED_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN" };
	const u32 count = QaMode() ? 1 : (u32)_countof(variables);
	for (u32 i = 0; i < count; ++i)
	{
		xr_string value = EnvText(variables[i]);
		if (PlausibleToken(value.c_str()))
		{
			out.source = asEnvironment;
			out.token = value;
			out.detail = variables[i];
			WipeString(value);
			return;
		}
	}
	if (ReadSavedToken(out.token))							{ out.source = asSaved; return; }
	if (!QaMode() && ReadGithubCliToken(out.token))			{ out.source = asGithubCli; return; }
	WipeString(out.token);
}

bool EditorGithub::SaveToken(LPCSTR token, xr_string& err)
{
	err.clear();
	if (!PlausibleToken(token)) { err = "this does not look like a GitHub token"; return false; }
	CREDENTIALW cred = {};
	cred.Type				= CRED_TYPE_GENERIC;
	cred.TargetName			= (LPWSTR)CredentialTarget();
	cred.UserName			= (LPWSTR)L"github.com";
	cred.CredentialBlobSize	= (DWORD)xr_strlen(token);
	cred.CredentialBlob		= (LPBYTE)token;
	cred.Persist			= CRED_PERSIST_LOCAL_MACHINE;
	if (::CredWriteW(&cred, 0)) return true;
	err = NqUtil::Format("Windows refused to store the token (error %u)", (u32)::GetLastError());
	return false;
}

void EditorGithub::ForgetToken()
{
	::CredDeleteW(CredentialTarget(), CRED_TYPE_GENERIC, 0);
}

//------------------------------------------------------------------------------
// device sign-in
//------------------------------------------------------------------------------
bool EditorGithub::DeviceSignInAvailable() { return !ClientId().empty(); }

bool EditorGithub::DeviceBegin(CCancel& cancel, SDeviceCode& out, xr_string& err)
{
	out = SDeviceCode();
	err.clear();
	CHttp http;
	// public_repo: a release the game can download lives in a public repository
	const SHttpResponse r = PostForm(http, cancel, WebBase() + "/login/device/code",
		"client_id=" + UrlEncode(ClientId()) + "&scope=public_repo");
	SJson j;
	if (r.status != 200 || !ParseJson(r.body, j) || !j.Str("device_code")[0] || !j.Str("user_code")[0])
	{
		err = r.status ? NqUtil::Format("GitHub refused to start the sign-in (%u)", r.status) : r.error;
		return false;
	}
	out.device_code			= j.Str("device_code");
	out.user_code			= j.Str("user_code");
	out.verification_uri	= j.Str("verification_uri");
	if (out.verification_uri.empty()) out.verification_uri = WebBase() + "/login/device";
	if (j.U64("interval"))		out.interval	= (u32)std::min<u64>(j.U64("interval"), 60);
	if (j.U64("expires_in"))	out.expires_in	= (u32)std::min<u64>(j.U64("expires_in"), 3600);
	return true;
}

int EditorGithub::DevicePoll(CCancel& cancel, SDeviceCode& code, xr_string& token, xr_string& err)
{
	err.clear();
	CHttp http;
	const SHttpResponse r = PostForm(http, cancel, WebBase() + "/login/oauth/access_token",
		"client_id=" + UrlEncode(ClientId()) + "&device_code=" + UrlEncode(code.device_code) +
		"&grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code");
	if (cancel.Cancelled()) { err = "cancelled"; return -1; }
	// a lost poll is not a lost sign-in: the code stays good until it expires
	if (!r.status) return 0;
	SJson j;
	if (r.status != 200 || !ParseJson(r.body, j)) { err = NqUtil::Format("GitHub answered %u to the sign-in", r.status); return -1; }

	if (PlausibleToken(j.Str("access_token")))
	{
		token = j.Str("access_token");
		return 1;
	}
	const xr_string problem = j.Str("error");
	if (problem == "authorization_pending") return 0;
	if (problem == "slow_down")
	{
		code.interval = (u32)std::min<u64>(std::max<u64>(j.U64("interval"), code.interval + 5), 60);
		return 0;
	}
	if (problem == "expired_token")		err = "The code expired. Start the sign-in again.";
	else if (problem == "access_denied")	err = "The sign-in was declined on GitHub.";
	else									err = "GitHub refused the sign-in: " + (problem.empty() ? xr_string("no reason given") : problem);
	return -1;
}

//------------------------------------------------------------------------------
// publication
//------------------------------------------------------------------------------
xr_string EditorGithub::ReleasesPage(LPCSTR repo)
{
	return WebBase() + "/" + repo + "/releases";
}

namespace
{
struct SRemoteAsset
{
	u64			id;
	xr_string	name;
	xr_string	state;
};

void ReadAssets(const SJson& release_or_list, xr_vector<SRemoteAsset>& out)
{
	out.clear();
	const SJson* list = (release_or_list.type == SJson::jArray) ? &release_or_list : release_or_list.Find("assets");
	if (!list || list->type != SJson::jArray) return;
	for (u32 i = 0; i < list->items.size(); ++i)
	{
		SRemoteAsset a;
		a.id	= list->items[i].U64("id");
		a.name	= list->items[i].Str("name");
		a.state	= list->items[i].Str("state");
		if (a.id) out.push_back(a);
	}
}

// names a release of this module + version puts on GitHub
bool OwnAssetName(const EditorGithub::SPublishRequest& rq, const xr_string& name)
{
	const xr_string stem = rq.module_id + "-" + rq.version + ".";
	return SameName(name.c_str(), (rq.module_id + ".update.ltx").c_str()) ||
		   (name.size() > stem.size() && 0 == _strnicmp(name.c_str(), stem.c_str(), stem.size()));
}

// the stored asset against what was packaged; empty = identical
xr_string AssetMismatch(const SJson& stored, const EditorGithub::SAsset& local)
{
	if (local.name != stored.Str("name"))
		return NqUtil::Format("stored under the name '%s'", stored.Str("name"));
	if (stored.U64("size") != local.size)
		return NqUtil::Format("%llu byte(s) stored, %llu packaged", stored.U64("size"), local.size);
	if (0 != xr_strcmp(stored.Str("state"), "uploaded"))
		return NqUtil::Format("state '%s'", stored.Str("state"));
	// older GitHub Enterprise has no digest; size and state still stand
	const xr_string digest = stored.Str("digest");
	if (!digest.empty() && !SameName(digest.c_str(), ("sha256:" + local.sha256).c_str()))
		return "the SHA-256 differs";
	return xr_string();
}
} // namespace

bool EditorGithub::Publish(const SPublishRequest& rq, CCancel& cancel, IPublishSink& sink,
						   SPublishResult& result, xr_string& err)
{
	result = SPublishResult();
	err.clear();
	CHttp http;
	if (!http.Ready()) { err = "WinHTTP is not available"; return false; }
	CApi api(http, cancel, rq.token);
	const xr_string repo_url = ApiBase() + "/repos/" + rq.repo;
	const xr_string descriptor_name = rq.module_id + ".update.ltx";
	const bool becomes_latest = rq.latest && !rq.prerelease;
	SJson j;

#define CHECK_CANCEL() if (cancel.Cancelled()) { err = "cancelled"; return false; }

	// 1. the repository, as this account sees it
	sink.Stage("Checking the repository");
	SHttpResponse r = api.Get(repo_url);
	CHECK_CANCEL();
	if (r.status == 401)
		err = "GitHub rejected the sign-in. Sign in again.";
	else if (r.status == 404)
		err = NqUtil::Format("Repository %s was not found, or this account cannot see it.", rq.repo.c_str());
	else if (r.status >= 300 && r.status < 400)
		err = NqUtil::Format("Repository %s has moved. Set its current name in Mod > Edit Manifest.", rq.repo.c_str());
	else if (r.status != 200 || !ParseJson(r.body, j))
		err = Problem(r);
	if (!err.empty()) return false;

	const bool is_private = j.Bool("private");
	const SJson* permissions = j.Find("permissions");
	if (j.Bool("archived"))
		{ err = NqUtil::Format("Repository %s is archived: it accepts no releases.", rq.repo.c_str()); return false; }
	if (permissions && permissions->Find("push") && !permissions->Bool("push"))
		{ err = NqUtil::Format("This account cannot write to %s.", rq.repo.c_str()); return false; }

	// 2. The game reads the latest release alone, so a module this release does
	// not carry would vanish for its players the moment this one becomes latest.
	if (becomes_latest)
	{
		r = api.Get(repo_url + "/releases/latest");
		CHECK_CANCEL();
		if (r.status == 200 && ParseJson(r.body, j))
		{
			xr_vector<SRemoteAsset> assets;
			ReadAssets(j, assets);
			for (u32 i = 0; i < assets.size(); ++i)
				if (EndsWith(assets[i].name, ".update.ltx") && !SameName(assets[i].name.c_str(), descriptor_name.c_str()))
				{
					err = NqUtil::Format("The latest release of %s serves another module (%s). This release would hide it "
						"from its players. Use a repository of its own for each module.", rq.repo.c_str(), assets[i].name.c_str());
					return false;
				}
		}
		else if (r.status != 404) { err = Problem(r); return false; }
	}

	// 3. a published release is never touched again
	r = api.Get(repo_url + "/releases/tags/" + UrlEncode(rq.tag));
	CHECK_CANCEL();
	if (r.status == 200)
	{
		err = NqUtil::Format("Release %s is already published in %s, and a published release is never overwritten. "
			"Raise the version - or, to put other files out under this one, delete that release in Releases first: "
			"the game then offers the new one to players who have it as a fix.", rq.tag.c_str(), rq.repo.c_str());
		return false;
	}
	if (r.status != 404) { err = Problem(r); return false; }

	// 4. a draft an earlier attempt left for this tag is picked up again
	u64 release_id = 0;
	xr_string upload_url;
	xr_vector<SRemoteAsset> leftovers;
	r = api.Get(repo_url + "/releases?per_page=100");
	CHECK_CANCEL();
	if (r.status != 200 || !ParseJson(r.body, j) || j.type != SJson::jArray) { err = Problem(r); return false; }
	for (u32 i = 0; i < j.items.size() && !release_id; ++i)
		if (j.items[i].Bool("draft") && rq.tag == j.items[i].Str("tag_name"))
		{
			release_id = j.items[i].U64("id");
			upload_url = j.items[i].Str("upload_url");
			ReadAssets(j.items[i], leftovers);
		}
	// The editor's tags for one version are v<version> and v<version>-r<N>: the suffix is how a
	// version goes out again under a fresh address, not a way around "this version is out".
	const xr_string version_tag = "v" + rq.version;
	for (u32 i = 0; i < j.items.size(); ++i)
	{
		const xr_string other = j.items[i].Str("tag_name");
		if (j.items[i].Bool("draft") || other == rq.tag || 0 != other.compare(0, version_tag.size(), version_tag))
			continue;
		LPCSTR rest = other.c_str() + version_tag.size();
		bool reissue_tag = '-' == rest[0] && 'r' == rest[1] && rest[2];
		for (LPCSTR c = rest + 2; reissue_tag && *c; ++c) reissue_tag = *c >= '0' && *c <= '9';
		if (rest[0] && !reissue_tag) continue;
		err = NqUtil::Format("Version %s is already published in %s as %s, and a published release is never "
			"overwritten. Raise the version - or, to put other files out under this one, delete that release in "
			"Releases first: the game then offers the new one to players who have it as a fix.",
			rq.version.c_str(), rq.repo.c_str(), other.c_str());
		return false;
	}

	// 5. the draft
	sink.Stage(release_id ? "Reusing the draft release" : "Creating a draft release");
	xr_string body = "{\"tag_name\":\"";
	JsonEscape(body, rq.tag);
	body += "\",\"name\":\"";
	JsonEscape(body, rq.title);
	body += "\",\"body\":\"";
	JsonEscape(body, rq.notes);
	body += NqUtil::Format("\",\"draft\":true,\"prerelease\":%s}", rq.prerelease ? "true" : "false");
	r = release_id	? api.Send("PATCH", repo_url + NqUtil::Format("/releases/%llu", release_id), body)
					: api.Send("POST", repo_url + "/releases", body);
	CHECK_CANCEL();
	if (r.status == 403 || r.status == 404)
	{
		err = NqUtil::Format("This sign-in cannot publish to %s. A fine-grained token needs the Contents permission "
			"set to Read and write; a classic one needs public_repo.", rq.repo.c_str());
		return false;
	}
	if ((r.status != 200 && r.status != 201) || !ParseJson(r.body, j) || !j.U64("id")) { err = Problem(r); return false; }
	release_id = j.U64("id");
	upload_url = j.Str("upload_url");
	const size_t brace = upload_url.find('{');
	if (brace != xr_string::npos) upload_url.erase(brace);
	if (!UploadUrlTrusted(upload_url)) { err = "GitHub named an upload address that is not GitHub's."; return false; }
	const xr_string release_url = repo_url + NqUtil::Format("/releases/%llu", release_id);

	// 6. what the earlier attempt left under this release's names
	for (u32 i = 0; i < leftovers.size(); ++i)
		if (OwnAssetName(rq, leftovers[i].name) || leftovers[i].state != "uploaded")
		{
			r = api.Delete(repo_url + NqUtil::Format("/releases/assets/%llu", leftovers[i].id));
			CHECK_CANCEL();
			if (r.status != 204 && r.status != 404) { err = Problem(r); return false; }
		}

	// 7. the assets
	for (u32 i = 0; i < rq.assets.size(); ++i)
	{
		const SAsset& asset = rq.assets[i];
		sink.Stage(NqUtil::Format("Uploading %s", asset.name.c_str()).c_str());
		xr_string failure;
		bool stored = false;
		for (u32 attempt = 0; attempt < kUploadAttempts && !stored; ++attempt)
		{
			if (attempt)
			{
				// a broken upload leaves a stub that owns the name
				Pause(cancel, 2000);
				r = api.Get(release_url + "/assets?per_page=100");
				CHECK_CANCEL();
				xr_vector<SRemoteAsset> now;
				if (r.status == 200 && ParseJson(r.body, j)) ReadAssets(j, now);
				for (u32 k = 0; k < now.size(); ++k)
					if (SameName(now[k].name.c_str(), asset.name.c_str()))
						api.Delete(repo_url + NqUtil::Format("/releases/assets/%llu", now[k].id));
				CHECK_CANCEL();
			}
			sink.Upload(asset.name.c_str(), 0, asset.size);
			r = api.Upload(upload_url + "?name=" + UrlEncode(asset.name), asset.path,
				[&](u64 sent, u64 total) { sink.Upload(asset.name.c_str(), sent, total); });
			CHECK_CANCEL();
			if (r.status == 201 && ParseJson(r.body, j))
			{
				const xr_string mismatch = AssetMismatch(j, asset);
				if (!mismatch.empty())
				{
					err = NqUtil::Format("GitHub did not store %s as it was packaged: %s. Nothing was published.",
						asset.name.c_str(), mismatch.c_str());
					return false;
				}
				stored = true;
			}
			else
			{
				failure = Problem(r);
				// a refusal will not change on a second try; a lost connection, a 5xx or a name clash may
				if (r.status >= 400 && r.status < 500 && r.status != 422 && r.status != 408 && r.status != 429) break;
			}
		}
		if (!stored)
		{
			err = NqUtil::Format("Uploading %s failed: %s. Nothing was published.", asset.name.c_str(), failure.c_str());
			return false;
		}
	}

	// 8. from here on players can see it
	sink.Stage("Publishing the release");
	body = NqUtil::Format("{\"draft\":false,\"prerelease\":%s,\"make_latest\":\"%s\"}",
		rq.prerelease ? "true" : "false", becomes_latest ? "true" : "false");
	r = api.Send("PATCH", release_url, body);
	if (r.status != 200 || !ParseJson(r.body, j))
	{
		// the request may have landed before the connection was cut
		err = cancel.Cancelled()	? "Cancelled while the release was being published. Check the release page: it may be out."
									: Problem(r);
		return false;
	}
	result.url = j.Str("html_url");
	if (result.url.empty()) result.url = ReleasesPage(rq.repo.c_str());

#undef CHECK_CANCEL

	// 9. the address the game itself asks
	if (!becomes_latest) return true;
	if (is_private)
	{
		result.warnings = "The repository is private. The game downloads releases anonymously, so players receive "
						  "nothing until the repository is public.";
		return true;
	}
	sink.Stage("Checking what the game will see");
	xr_string local;
	ReadWholeFile(rq.descriptor.c_str(), local);
	for (u32 attempt = 0; attempt < kServeAttempts && !result.served && !cancel.Cancelled(); ++attempt)
	{
		if (attempt) Pause(cancel, 3000);
		SHttpRequest check;
		check.url = WebBase() + "/" + rq.repo + "/releases/latest/download/" + descriptor_name;
		check.redirects = true;
		const SHttpResponse served = http.Send(check, cancel, TUploadProgress());
		result.served = served.status == 200 && !local.empty() && served.body == local;
	}
	if (!result.served)
		// Seen live: a release deleted and published again under the same tag keeps its download
		// address, and GitHub goes on answering that address with the deleted files for minutes.
		result.warnings = "GitHub still answers the address the game asks with the previous files. That is GitHub's "
						  "own cache - it happens when a tag is used a second time - and it passes within minutes; "
						  "the game offers this release after that.";
	return true;
}

//------------------------------------------------------------------------------
// listing and deleting
//------------------------------------------------------------------------------
namespace
{
// 2026-09-19T12:34:56Z -> 2026-09-19 12:34
xr_string ShortTime(LPCSTR iso)
{
	xr_string s = iso;
	if (s.size() < 16 || s[10] != 'T') return s;
	s[10] = ' ';
	return s.substr(0, 16);
}

// a ref keeps its slashes; everything between them is escaped
xr_string EncodeRef(const xr_string& ref)
{
	xr_string out;
	size_t from = 0;
	for (;;)
	{
		const size_t slash = ref.find('/', from);
		out += UrlEncode(ref.substr(from, slash == xr_string::npos ? xr_string::npos : slash - from));
		if (slash == xr_string::npos) return out;
		out += '/';
		from = slash + 1;
	}
}

// the refusals that mean the same thing whatever was asked
xr_string AccessProblem(const SHttpResponse& r, const xr_string& repo)
{
	if (r.status == 401) return "GitHub rejected the sign-in. Sign in again.";
	if (r.status >= 300 && r.status < 400)
		return NqUtil::Format("Repository %s has moved. Set its current name in Mod > Edit Manifest.", repo.c_str());
	return Problem(r);
}
} // namespace

bool EditorGithub::ListReleases(const xr_string& repo, const xr_string& token, CCancel& cancel,
								xr_vector<SReleaseInfo>& out, xr_string& err)
{
	out.clear();
	err.clear();
	CHttp http;
	if (!http.Ready()) { err = "WinHTTP is not available"; return false; }
	CApi api(http, cancel, token);
	const xr_string repo_url = ApiBase() + "/repos/" + repo;
	SJson j;

	// the list does not say which release is the latest; this does
	u64 latest_id = 0;
	SHttpResponse r = api.Get(repo_url + "/releases/latest");
	if (cancel.Cancelled()) { err = "cancelled"; return false; }
	if (r.status == 200 && ParseJson(r.body, j))	latest_id = j.U64("id");
	// 404 is "no latest release" as much as "no such repository"; the list below tells them apart
	else if (r.status != 404)						{ err = AccessProblem(r, repo); return false; }

	for (u32 page = 1; page <= 10; ++page)
	{
		r = api.Get(repo_url + NqUtil::Format("/releases?per_page=100&page=%u", page));
		if (cancel.Cancelled()) { err = "cancelled"; return false; }
		if (r.status == 404)
		{
			err = NqUtil::Format("Repository %s was not found, or this account cannot see it.", repo.c_str());
			return false;
		}
		if (r.status != 200 || !ParseJson(r.body, j) || j.type != SJson::jArray) { err = AccessProblem(r, repo); return false; }
		for (u32 i = 0; i < j.items.size(); ++i)
		{
			const SJson& item = j.items[i];
			SReleaseInfo info;
			info.id			= item.U64("id");
			info.tag		= item.Str("tag_name");
			info.title		= item.Str("name");
			info.notes		= item.Str("body");
			info.url		= item.Str("html_url");
			info.draft		= item.Bool("draft");
			info.prerelease	= item.Bool("prerelease");
			info.published	= info.draft ? xr_string() : ShortTime(item.Str("published_at"));
			info.latest		= info.id && info.id == latest_id;
			const SJson* assets = item.Find("assets");
			if (assets && assets->type == SJson::jArray)
				for (u32 a = 0; a < assets->items.size(); ++a)
				{
					info.assets.push_back(assets->items[a].Str("name"));
					info.bytes += assets->items[a].U64("size");
				}
			if (info.id) out.push_back(info);
		}
		if (j.items.size() < 100) break;
	}
	return true;
}

bool EditorGithub::EditRelease(const xr_string& repo, const xr_string& token, u64 id, const xr_string* title,
							   const xr_string* notes, CCancel& cancel, xr_string& tag, xr_string& err)
{
	tag.clear();
	err.clear();
	CHttp http;
	if (!http.Ready()) { err = "WinHTTP is not available"; return false; }
	CApi api(http, cancel, token);
	const xr_string repo_url = ApiBase() + "/repos/" + repo;
	const xr_string release_url = repo_url + NqUtil::Format("/releases/%llu", id);
	SJson j;

	SHttpResponse r = api.Get(release_url);
	if (cancel.Cancelled()) { err = "cancelled"; return false; }
	if (r.status == 404) { err = "The release no longer exists."; return false; }
	if (r.status != 200 || !ParseJson(r.body, j)) { err = AccessProblem(r, repo); return false; }
	tag = j.Str("tag_name");
	const bool can_be_latest = !j.Bool("draft") && !j.Bool("prerelease");

	// where the mark is now - see the header
	bool is_latest = false;
	if (can_be_latest)
	{
		r = api.Get(repo_url + "/releases/latest");
		if (cancel.Cancelled()) { err = "cancelled"; return false; }
		SJson latest;
		if (r.status == 200 && ParseJson(r.body, latest))	is_latest = latest.U64("id") == id;
		else if (r.status != 404)							{ err = AccessProblem(r, repo); return false; }
	}

	xr_string body = "{";
	if (title)
	{
		body += "\"name\":\"";
		JsonEscape(body, *title);
		body += "\",";
	}
	if (notes)
	{
		body += "\"body\":\"";
		JsonEscape(body, *notes);
		body += "\",";
	}
	body += NqUtil::Format("\"make_latest\":\"%s\"}", is_latest ? "true" : "false");
	r = api.Send("PATCH", release_url, body);
	if (r.status == 403)
	{
		err = NqUtil::Format("This sign-in cannot edit releases in %s. A fine-grained token needs the Contents "
			"permission set to Read and write; a classic one needs public_repo.", repo.c_str());
		return false;
	}
	if (r.status != 200)
	{
		err = cancel.Cancelled() ? "Cancelled while the release was being saved. Refresh the list to see what it holds."
								 : AccessProblem(r, repo);
		return false;
	}
	return true;
}

bool EditorGithub::DeleteRelease(const xr_string& repo, const xr_string& token, u64 id, CCancel& cancel,
								 SDeleteResult& result, xr_string& err)
{
	result = SDeleteResult();
	err.clear();
	CHttp http;
	if (!http.Ready()) { err = "WinHTTP is not available"; return false; }
	CApi api(http, cancel, token);
	const xr_string repo_url = ApiBase() + "/repos/" + repo;
	const xr_string release_url = repo_url + NqUtil::Format("/releases/%llu", id);
	SJson j;

	// as it is now, not as a list drawn a minute ago had it
	SHttpResponse r = api.Get(release_url);
	if (cancel.Cancelled()) { err = "cancelled"; return false; }
	if (r.status == 404) { err = "The release no longer exists."; return false; }
	if (r.status != 200 || !ParseJson(r.body, j)) { err = AccessProblem(r, repo); return false; }
	result.tag = j.Str("tag_name");
	result.draft = j.Bool("draft");

	r = api.Delete(release_url);
	if (r.status == 404) { err = "The release no longer exists."; return false; }
	if (r.status == 403)
	{
		err = NqUtil::Format("This sign-in cannot delete releases in %s. A fine-grained token needs the Contents "
			"permission set to Read and write; a classic one needs public_repo.", repo.c_str());
		return false;
	}
	if (r.status != 204)
	{
		// the request may have landed before the connection was cut
		err = cancel.Cancelled() ? "Cancelled while the release was being deleted. Refresh the list: it may be gone."
								 : AccessProblem(r, repo);
		return false;
	}

	if (result.draft || result.tag.empty()) return true;
	r = api.Delete(repo_url + "/git/refs/tags/" + EncodeRef(result.tag));
	if (r.status == 204)
		result.tag_removed = true;
	// "Reference does not exist": nothing to remove is as good as removed
	else if (r.status == 404 || (r.status == 422 && xr_string::npos != r.body.find("does not exist")))
		result.tag_removed = true;
	else
		result.tag_note = cancel.Cancelled() ? xr_string("cancelled before the tag was removed") : Problem(r);
	return true;
}

//------------------------------------------------------------------------------
// public files
//------------------------------------------------------------------------------
bool EditorGithub::FetchPublic(const xr_string& url, u64 limit, CCancel& cancel, u32& status, xr_string& body,
							   xr_string& err)
{
	status = 0;
	body.clear();
	err.clear();
	CHttp http;
	if (!http.Ready()) { err = "WinHTTP is not available"; return false; }
	SHttpRequest rq;
	rq.url = url;
	rq.redirects = true;
	SHttpResponse r = http.Send(rq, cancel, TUploadProgress());
	if (!r.status) { err = r.error; return false; }
	status = r.status;
	if (r.body.size() > limit) { err = NqUtil::Format("%s is larger than %llu bytes", url.c_str(), limit); return false; }
	body.swap(r.body);
	return true;
}

xr_string EditorGithub::LatestDownload(const xr_string& repo, const xr_string& name)
{
	return WebBase() + "/" + repo + "/releases/latest/download/" + UrlEncode(name);
}

xr_string EditorGithub::RepoPage(const xr_string& repo) { return WebBase() + "/" + repo; }

//------------------------------------------------------------------------------
// the Mod Browser catalog
//------------------------------------------------------------------------------
xr_string EditorGithub::CatalogRepo()
{
	const xr_string qa = QaMode() ? EnvText("XFINED_CATALOG_REPO") : xr_string();
	return qa.empty() ? xr_string(kCatalogRepo) : qa;
}

void EditorGithub::CatalogMirrors(xr_vector<xr_string>& out)
{
	out.clear();
	if (QaMode())
	{
		// loopback only, like the game's own QA override
		const xr_string base = EnvText("XFINED_CATALOG_BASE");
		if ((0 == base.compare(0, 17, "http://127.0.0.1:") || 0 == base.compare(0, 17, "http://localhost:")) &&
			!base.empty() && base.back() == '/')
			out.push_back(base);
		return;
	}
	for (u32 i = 0; i < _countof(kCatalogMirrors); ++i)
		out.push_back(kCatalogMirrors[i]);
}

bool EditorGithub::AccountLogin(const xr_string& token, CCancel& cancel, xr_string& login, xr_string& err)
{
	login.clear();
	err.clear();
	CHttp http;
	if (!http.Ready()) { err = "WinHTTP is not available"; return false; }
	CApi api(http, cancel, token);
	const SHttpResponse r = api.Get(ApiBase() + "/user");
	if (cancel.Cancelled())		{ err = "cancelled"; return false; }
	if (r.status == 401)		{ err = "GitHub rejected the sign-in. Sign in again."; return false; }
	SJson j;
	if (r.status != 200 || !ParseJson(r.body, j) || !j.Str("login")[0]) { err = Problem(r); return false; }
	login = j.Str("login");
	return true;
}

namespace
{
// base64 with the line breaks the contents API puts in
bool UnBase64(const xr_string& text, xr_string& out)
{
	out.clear();
	u32 buffer = 0, bits = 0;
	for (size_t i = 0; i < text.size(); ++i)
	{
		const char c = text[i];
		int v;
		if (c >= 'A' && c <= 'Z')		v = c - 'A';
		else if (c >= 'a' && c <= 'z')	v = c - 'a' + 26;
		else if (c >= '0' && c <= '9')	v = c - '0' + 52;
		else if (c == '+')				v = 62;
		else if (c == '/')				v = 63;
		else if (c == '=' || c == '\n' || c == '\r' || c == ' ') continue;
		else return false;
		buffer = (buffer << 6) | u32(v);
		bits += 6;
		if (bits >= 8)
		{
			bits -= 8;
			out += char((buffer >> bits) & 0xFF);
		}
	}
	return true;
}

// the account the token belongs to
bool ReadLogin(CApi& api, EditorGithub::CCancel& cancel, xr_string& login, xr_string& err)
{
	const SHttpResponse r = api.Get(ApiBase() + "/user");
	SJson j;
	if (cancel.Cancelled())	{ err = "cancelled"; return false; }
	if (r.status == 401)	{ err = "GitHub rejected the sign-in. Sign in again."; return false; }
	if (r.status != 200 || !ParseJson(r.body, j) || !j.Str("login")[0]) { err = Problem(r); return false; }
	login = j.Str("login");
	return true;
}

void ReadIssue(const SJson& j, EditorGithub::SCatalogIssue& out)
{
	out.number	= j.U64("number");
	out.url		= j.Str("html_url");
	out.state	= j.Str("state");
	if (out.state.empty()) out.state = "open";
}

// The account's newest "Submit <id>" issue of the catalog, in any state; `out` keeps what it has
// when there is none. The list is newest first and holds the account's pull requests as well.
bool FindIssue(CApi& api, EditorGithub::CCancel& cancel, const xr_string& catalog, const xr_string& login,
			   const xr_string& module_id, EditorGithub::SCatalogIssue& out, xr_string& err)
{
	const xr_string title = "Submit " + module_id;
	const u32 per_page = 100, pages = 10;
	for (u32 page = 1; page <= pages; ++page)
	{
		const SHttpResponse r = api.Get(ApiBase() + "/repos/" + catalog + "/issues?creator=" + UrlEncode(login) +
			NqUtil::Format("&state=all&per_page=%u&page=%u", per_page, page));
		if (cancel.Cancelled()) { err = "cancelled"; return false; }
		SJson j;
		if (r.status == 404) { err = "The Mod Browser catalog " + catalog + " was not found."; return false; }
		if (r.status != 200 || !ParseJson(r.body, j) || j.type != SJson::jArray) { err = AccessProblem(r, catalog); return false; }
		for (u32 i = 0; i < j.items.size(); ++i)
			if (!j.items[i].Find("pull_request") && title == j.items[i].Str("title"))
			{
				ReadIssue(j.items[i], out);
				return true;
			}
		if (j.items.size() < per_page) break;
	}
	return true;
}

bool Blank(char c) { return c == ' ' || c == '\t' || c == '\r'; }

// A verdict comment: "<!-- xms-catalog verdict=STATE -->" on its first line, one plain sentence on
// the next line that holds anything, then a report for people. Spacing and line ends are the writer's.
bool ParseVerdict(const xr_string& body, xr_string& state, xr_string& text)
{
	state.clear();
	text.clear();
	xr_string line[2];
	u32 lines = 0;
	for (size_t at = 0; at < body.size() && lines < 2;)
	{
		size_t end = body.find('\n', at);
		if (end == xr_string::npos) end = body.size();
		size_t first = at, last = end;
		while (first < last && Blank(body[first])) ++first;
		while (last > first && Blank(body[last - 1])) --last;
		if (first < last) line[lines++] = body.substr(first, last - first);
		at = end + 1;
	}
	const xr_string& marker = line[0];
	if (marker.size() < 7 || 0 != marker.compare(0, 4, "<!--") || 0 != marker.compare(marker.size() - 3, 3, "-->"))
		return false;
	const char* p = marker.c_str() + 4;
	const char* const stop = marker.c_str() + marker.size() - 3;
	const auto skip = [&]() { while (p < stop && Blank(*p)) ++p; };
	const auto take = [&](LPCSTR word)
	{
		const size_t n = xr_strlen(word);
		if ((size_t)(stop - p) < n || 0 != strncmp(p, word, n)) return false;
		p += n;
		return true;
	};
	skip();
	if (!take("xms-catalog") || p == stop || !Blank(*p)) return false;
	skip();
	if (!take("verdict")) return false;
	skip();
	if (!take("=")) return false;
	skip();
	xr_string word;
	while (p < stop && !Blank(*p)) word += char(tolower((unsigned char)*p++));
	if (word != "accepted" && word != "waiting" && word != "refused") return false;
	state = word;
	text = line[1];
	return true;
}

// The comments come oldest first, a page at a time, and the newest verdict of the catalog's workflow
// is the one in force; anyone may post a comment that looks like one. false = cancelled. A failed
// page, or more comments than the pages hold, leaves the submission without a verdict rather than
// with an old one.
bool ReadVerdict(CApi& api, EditorGithub::CCancel& cancel, const xr_string& catalog, EditorGithub::SCatalogIssue& out)
{
	const u32 per_page = 100, pages = 30;
	for (u32 page = 1; page <= pages; ++page)
	{
		const SHttpResponse r = api.Get(ApiBase() + "/repos/" + catalog +
			NqUtil::Format("/issues/%llu/comments?per_page=%u&page=%u", out.number, per_page, page));
		if (cancel.Cancelled()) return false;
		SJson j;
		if (r.status != 200 || !ParseJson(r.body, j) || j.type != SJson::jArray) break;
		for (u32 i = 0; i < j.items.size(); ++i)
		{
			const SJson* user = j.items[i].Find("user");
			xr_string state, text;
			if (user && 0 == strcmp(user->Str("login"), kCatalogBot) && ParseVerdict(j.items[i].Str("body"), state, text))
			{
				out.verdict = state;
				out.verdict_text = text;
			}
		}
		if (j.items.size() < per_page) return true;
	}
	out.verdict.clear();
	out.verdict_text.clear();
	return true;
}

// The issue's body: the marker, a line for people, then the file the catalog commits in an ini
// block of its own - byte for byte, the catalog takes it from there - and the details after it.
xr_string SubmissionBody(const EditorGithub::SCatalogRequest& rq)
{
	xr_string body = "<!-- xms-catalog submission -->\n"
		"Submitted from XFined Editor. The catalog's own checks judge it (MOD_CATALOG.md, section 7).\n\n```ini\n" +
		rq.file_text;
	if (rq.file_text.empty() || rq.file_text.back() != '\n') body += '\n';
	body += "```\n";
	if (!rq.details.empty()) body += "\n" + rq.details;
	return body;
}
} // namespace

bool EditorGithub::CatalogSubmit(const SCatalogRequest& rq, CCancel& cancel, const TStage& stage, SCatalogIssue& out,
								 xr_string& err)
{
	out = SCatalogIssue();
	err.clear();
	CHttp http;
	if (!http.Ready()) { err = "WinHTTP is not available"; return false; }
	CApi api(http, cancel, rq.token);
	const xr_string catalog = CatalogRepo();
	const xr_string catalog_url = ApiBase() + "/repos/" + catalog;
	const auto say = [&stage](LPCSTR text) { if (stage) stage(text); };

	say("Reading the GitHub account");
	if (!ReadLogin(api, cancel, out.login, err)) return false;
	say("Reading the submission");
	if (!FindIssue(api, cancel, catalog, out.login, rq.module_id, out, err)) return false;

	// Nothing to send when the catalog's default branch already holds this very file - a module
	// listed and submitted again unchanged.
	SHttpResponse r = api.Get(catalog_url + "/contents/mods/" + rq.module_id + ".ltx");
	if (cancel.Cancelled()) { err = "cancelled"; return false; }
	SJson j;
	xr_string current;
	if (r.status == 200 && ParseJson(r.body, j) && UnBase64(j.Str("content"), current) && current == rq.file_text)
	{
		out.note = "The catalog already holds this exact submission - nothing was sent.";
		// a closed submission is told apart by its verdict: what the catalog said about the last one
		if (out.number && !ReadVerdict(api, cancel, catalog, out)) { err = "cancelled"; return false; }
		return true;
	}

	// The open submission takes the new text, and the edit makes the catalog judge it again. Once the
	// last one is closed - accepted or refused - a new one opens.
	xr_string body = "{\"body\":\"";
	JsonEscape(body, SubmissionBody(rq));
	body += "\"";
	if (out.number && out.state == "open")
	{
		say("Updating the submission");
		r = api.Send("PATCH", catalog_url + NqUtil::Format("/issues/%llu", out.number), body + "}");
	}
	else
	{
		say("Opening the submission");
		body += ",\"title\":\"";
		JsonEscape(body, "Submit " + rq.module_id);
		r = api.Send("POST", catalog_url + "/issues", body + "\"}");
	}
	if (cancel.Cancelled()) { err = "cancelled"; return false; }
	if ((r.status != 200 && r.status != 201) || !ParseJson(r.body, j) || !j.U64("number"))
		{ err = "GitHub refused the submission: " + AccessProblem(r, catalog); return false; }
	ReadIssue(j, out);
	return true;
}

bool EditorGithub::CatalogIssue(const xr_string& module_id, const xr_string& token, CCancel& cancel, SCatalogIssue& out,
								xr_string& err)
{
	out = SCatalogIssue();
	err.clear();
	CHttp http;
	if (!http.Ready()) { err = "WinHTTP is not available"; return false; }
	CApi api(http, cancel, token);
	const xr_string catalog = CatalogRepo();
	if (!ReadLogin(api, cancel, out.login, err)) return false;
	if (!FindIssue(api, cancel, catalog, out.login, module_id, out, err)) return false;
	if (out.number && !ReadVerdict(api, cancel, catalog, out)) { err = "cancelled"; return false; }
	return true;
}

//------------------------------------------------------------------------------
// VirusTotal
//------------------------------------------------------------------------------
namespace
{
const wchar_t* VtCredentialTarget()
{
	return QaMode() ? L"XFinedEditor:VirusTotal:qa" : L"XFinedEditor:VirusTotal";
}

// VirusTotal answers {"error":{"code","message"}}
xr_string VtProblem(const SHttpResponse& r)
{
	if (!r.status) return r.error;
	if (r.status == 401) return "VirusTotal rejected the key";
	if (r.status == 429) return "VirusTotal's free API takes 4 requests a minute and 500 a day - try again later";
	SJson j;
	const SJson* error = ParseJson(r.body, j) ? j.Find("error") : 0;
	const xr_string text = error ? error->Str("message") : "";
	return text.empty() ? NqUtil::Format("VirusTotal answered %u", r.status)
						: NqUtil::Format("VirusTotal answered %u: %s", r.status, text.c_str());
}

SHttpResponse VtCall(CHttp& http, EditorGithub::CCancel& cancel, LPCSTR method, const xr_string& url, const xr_string& key)
{
	SHttpRequest rq;
	rq.method = method;
	rq.url = url;
	rq.headers.push_back("Accept: application/json");
	rq.headers.push_back("x-apikey: " + key);
	SHttpResponse r = http.Send(rq, cancel, TUploadProgress());
	EditorGithub::WipeString(rq.headers[1]);
	return r;
}
} // namespace

bool EditorGithub::PlausibleVtKey(LPCSTR key)
{
	const size_t n = key ? xr_strlen(key) : 0;
	if (n < 8 || n > 128) return false;
	for (size_t i = 0; i < n; ++i)
	{
		const char c = key[i];
		if (!((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_' || c == '-'))
			return false;
	}
	return true;
}

bool EditorGithub::ReadVtKey(xr_string& key)
{
	// an explicit override first; a QA run reads its own variable and credential only
	key = EnvText("XFINED_VT_KEY");
	if (key.empty() && !QaMode()) key = EnvText("VT_API_KEY");
	if (PlausibleVtKey(key.c_str())) return true;
	WipeString(key);
	PCREDENTIALW cred = 0;
	if (!::CredReadW(VtCredentialTarget(), CRED_TYPE_GENERIC, 0, &cred)) return false;
	key.assign((const char*)cred->CredentialBlob, cred->CredentialBlobSize);
	SecureZeroMemory(cred->CredentialBlob, cred->CredentialBlobSize);
	::CredFree(cred);
	if (PlausibleVtKey(key.c_str())) return true;
	WipeString(key);
	return false;
}

bool EditorGithub::SaveVtKey(LPCSTR key, xr_string& err)
{
	err.clear();
	if (!PlausibleVtKey(key)) { err = "this does not look like a VirusTotal API key"; return false; }
	CREDENTIALW cred = {};
	cred.Type				= CRED_TYPE_GENERIC;
	cred.TargetName			= (LPWSTR)VtCredentialTarget();
	cred.UserName			= (LPWSTR)L"virustotal.com";
	cred.CredentialBlobSize	= (DWORD)xr_strlen(key);
	cred.CredentialBlob		= (LPBYTE)key;
	cred.Persist			= CRED_PERSIST_LOCAL_MACHINE;
	if (::CredWriteW(&cred, 0)) return true;
	err = NqUtil::Format("Windows refused to store the key (error %u)", (u32)::GetLastError());
	return false;
}

void EditorGithub::ForgetVtKey()
{
	::CredDeleteW(VtCredentialTarget(), CRED_TYPE_GENERIC, 0);
}

xr_string EditorGithub::VtReportPage(const xr_string& sha256)
{
	return "https://www.virustotal.com/gui/file/" + sha256;
}

bool EditorGithub::VtLookup(const xr_string& sha256, const xr_string& key, CCancel& cancel, SVtReport& out, xr_string& err)
{
	out = SVtReport();
	err.clear();
	CHttp http;
	if (!http.Ready()) { err = "WinHTTP is not available"; return false; }
	const SHttpResponse r = VtCall(http, cancel, "GET", VtBase() + "/files/" + sha256, key);
	out.status = r.status;
	if (cancel.Cancelled())	{ err = "cancelled"; return false; }
	if (r.status == 404)	return true;
	SJson j;
	const SJson* data = (r.status == 200 && ParseJson(r.body, j)) ? j.Find("data") : 0;
	const SJson* attributes = data ? data->Find("attributes") : 0;
	if (!attributes) { err = VtProblem(r); return false; }
	out.known = true;
	const SJson* results = attributes->Find("last_analysis_results");
	if (results && results->type == SJson::jObject)
		for (u32 i = 0; i < results->items.size(); ++i)
		{
			SVtResult result;
			result.engine	= results->keys[i];
			result.category	= results->items[i].Str("category");
			out.results.push_back(result);
		}
	// an upload VirusTotal has not analysed yet is known without a verdict
	out.queued = out.results.empty();
	return true;
}

bool EditorGithub::VtUpload(const xr_string& path, const xr_string& key, CCancel& cancel, u32& status, xr_string& err)
{
	status = 0;
	err.clear();
	CHttp http;
	if (!http.Ready()) { err = "WinHTTP is not available"; return false; }
	WIN32_FILE_ATTRIBUTE_DATA attributes;
	if (!::GetFileAttributesExA(path.c_str(), GetFileExInfoStandard, &attributes)) { err = "cannot read " + path; return false; }
	const u64 size = (u64(attributes.nFileSizeHigh) << 32) | attributes.nFileSizeLow;
	if (VtTooLarge(size)) { err = "the file is larger than the 650 MB VirusTotal takes"; return false; }

	// up to 32 MB straight to /files; a larger file to an address VirusTotal hands out for it
	// (a QA run lowers the line to reach that path with a small package)
	const xr_string qa_direct = QaMode() ? EnvText("XFINED_VT_DIRECT_LIMIT") : xr_string();
	const u64 direct = qa_direct.empty() ? (32ull << 20) : _strtoui64(qa_direct.c_str(), 0, 10);
	xr_string url = VtBase() + "/files";
	if (size + kVtWrapping > direct)
	{
		const SHttpResponse r = VtCall(http, cancel, "GET", VtBase() + "/files/upload_url", key);
		status = r.status;
		if (cancel.Cancelled()) { err = "cancelled"; return false; }
		SJson j;
		const SJson* data = (r.status == 200 && ParseJson(r.body, j)) ? j.Find("data") : 0;
		if (!data || data->type != SJson::jString) { err = VtProblem(r); return false; }
		url = data->text;
		// the key goes wherever that address points, and it came out of a response body
		if (0 != _stricmp(Origin(url).c_str(), Origin(VtBase()).c_str()))
			{ err = "VirusTotal named an upload address that is not VirusTotal's."; return false; }
	}

	u8 random[12];
	::BCryptGenRandom(0, random, sizeof(random), BCRYPT_USE_SYSTEM_PREFERRED_RNG);
	xr_string boundary = "XFinedEditor";
	for (u32 i = 0; i < sizeof(random); ++i) boundary += NqUtil::Format("%02x", (u32)random[i]);
	xr_string name = path.substr(path.find_last_of("\\/") + 1);
	for (size_t i = 0; i < name.size(); ++i)
		if (name[i] == '"' || (unsigned char)name[i] < 0x20) name[i] = '_';

	SHttpRequest rq;
	rq.method = "POST";
	rq.url = url;
	rq.headers.push_back("Accept: application/json");
	rq.headers.push_back("x-apikey: " + key);
	rq.headers.push_back("Content-Type: multipart/form-data; boundary=" + boundary);
	rq.body_file = path;
	rq.body_prefix = "--" + boundary + "\r\nContent-Disposition: form-data; name=\"file\"; filename=\"" + name +
		"\"\r\nContent-Type: application/octet-stream\r\n\r\n";
	rq.body_suffix = "\r\n--" + boundary + "--\r\n";
	const SHttpResponse r = http.Send(rq, cancel, TUploadProgress());
	WipeString(rq.headers[1]);
	status = r.status;
	if (cancel.Cancelled()) { err = "cancelled"; return false; }
	SJson j;
	const SJson* data = (r.status == 200 && ParseJson(r.body, j)) ? j.Find("data") : 0;
	if (!data || !data->Str("id")[0]) { err = VtProblem(r); return false; }
	return true;
}
