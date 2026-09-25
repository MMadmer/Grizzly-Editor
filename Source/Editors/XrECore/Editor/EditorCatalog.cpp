#include "stdafx.h"
#include "EditorCatalog.h"
#include "EditorProject.h"
#include "ImageManager.h"
#include "Nq/NqUtil.h"
#include "StbImage\stb_image.h"
#include <bcrypt.h>
#include <shlobj.h>
#include <atomic>
#include <mutex>

#pragma comment(lib, "bcrypt.lib")

namespace
{
const u32	kKdfRounds		= 600000;
const u64	kKeyFileLimit	= 16u << 10;
const u64	kThumbnailLimit	= 256u << 10;

//------------------------------------------------------------------------------
// bytes and text
//------------------------------------------------------------------------------
typedef xr_vector<u8> TBytes;

xr_string B64u(const u8* data, size_t size)
{
	static const char alphabet[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
	xr_string out;
	for (size_t i = 0; i < size; i += 3)
	{
		const u32 chunk = (u32(data[i]) << 16) | (i + 1 < size ? u32(data[i + 1]) << 8 : 0) | (i + 2 < size ? data[i + 2] : 0);
		out += alphabet[(chunk >> 18) & 63];
		out += alphabet[(chunk >> 12) & 63];
		if (i + 1 < size) out += alphabet[(chunk >> 6) & 63];
		if (i + 2 < size) out += alphabet[chunk & 63];
	}
	return out;
}

bool UnB64u(const xr_string& text, TBytes& out)
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
		else if (c == '-' || c == '+')	v = 62;
		else if (c == '_' || c == '/')	v = 63;
		else if (c == '=' || c == ' ' || c == '\r' || c == '\n') continue;
		else return false;
		buffer = (buffer << 6) | u32(v);
		bits += 6;
		if (bits >= 8)
		{
			bits -= 8;
			out.push_back(u8((buffer >> bits) & 0xFF));
		}
	}
	return true;
}

xr_string Hex(const u8* data, size_t size)
{
	static const char digits[] = "0123456789abcdef";
	xr_string out;
	for (size_t i = 0; i < size; ++i)
	{
		out += digits[data[i] >> 4];
		out += digits[data[i] & 15];
	}
	return out;
}

void Wipe(void* data, size_t size) { ::SecureZeroMemory(data, size); }

//------------------------------------------------------------------------------
// CNG: SHA-256, HMAC, PBKDF2, P-256
//------------------------------------------------------------------------------
bool Hash(LPCWSTR algorithm, bool hmac, const void* key, u32 key_size, const void* a, size_t a_size,
		  const void* b, size_t b_size, u8 (&out)[32])
{
	BCRYPT_ALG_HANDLE alg = 0;
	BCRYPT_HASH_HANDLE hash = 0;
	bool ok = BCRYPT_SUCCESS(::BCryptOpenAlgorithmProvider(&alg, algorithm, 0, hmac ? BCRYPT_ALG_HANDLE_HMAC_FLAG : 0)) &&
		BCRYPT_SUCCESS(::BCryptCreateHash(alg, &hash, 0, 0, (PUCHAR)key, key_size, 0)) &&
		BCRYPT_SUCCESS(::BCryptHashData(hash, (PUCHAR)a, (ULONG)a_size, 0)) &&
		(!b_size || BCRYPT_SUCCESS(::BCryptHashData(hash, (PUCHAR)b, (ULONG)b_size, 0))) &&
		BCRYPT_SUCCESS(::BCryptFinishHash(hash, out, 32, 0));
	if (hash) ::BCryptDestroyHash(hash);
	if (alg) ::BCryptCloseAlgorithmProvider(alg, 0);
	return ok;
}

bool Sha256(const void* data, size_t size, u8 (&out)[32])
{
	return Hash(BCRYPT_SHA256_ALGORITHM, false, 0, 0, data, size, 0, 0, out);
}

bool Hmac(const u8* key, u32 key_size, const void* a, size_t a_size, const void* b, size_t b_size, u8 (&out)[32])
{
	return Hash(BCRYPT_SHA256_ALGORITHM, true, key, key_size, a, a_size, b, b_size, out);
}

bool Pbkdf2(const xr_string& password, const TBytes& salt, u32 rounds, u8 (&out)[64])
{
	BCRYPT_ALG_HANDLE alg = 0;
	bool ok = BCRYPT_SUCCESS(::BCryptOpenAlgorithmProvider(&alg, BCRYPT_SHA256_ALGORITHM, 0, BCRYPT_ALG_HANDLE_HMAC_FLAG)) &&
		BCRYPT_SUCCESS(::BCryptDeriveKeyPBKDF2(alg, (PUCHAR)password.data(), (ULONG)password.size(), (PUCHAR)salt.data(),
			(ULONG)salt.size(), rounds, out, 64, 0));
	if (alg) ::BCryptCloseAlgorithmProvider(alg, 0);
	return ok;
}

bool Random(u8* out, u32 size)
{
	return BCRYPT_SUCCESS(::BCryptGenRandom(0, out, size, BCRYPT_USE_SYSTEM_PREFERRED_RNG));
}

// The key stream of the key file (MOD_CATALOG 4.1): block i is HMAC(key, nonce || i, 4 bytes BE).
bool Stream(const u8* key, const TBytes& nonce, u8* data, size_t size)
{
	for (size_t at = 0, block = 0; at < size; ++block)
	{
		const u8 counter[4] = { u8(block >> 24), u8(block >> 16), u8(block >> 8), u8(block) };
		u8 pad[32];
		if (!Hmac(key, 32, nonce.data(), nonce.size(), counter, 4, pad)) return false;
		for (size_t i = 0; i < 32 && at < size; ++i, ++at)
			data[at] ^= pad[i];
		Wipe(pad, sizeof(pad));
	}
	return true;
}

struct SEcKey
{
	u8	x[32], y[32], d[32];
	bool has_private;

	SEcKey() : has_private(false) { Wipe(this, sizeof(*this)); }
	~SEcKey() { Wipe(this, sizeof(*this)); }

	xr_string PublicText() const
	{
		u8 xy[64];
		memcpy(xy, x, 32);
		memcpy(xy + 32, y, 32);
		return "p256:" + B64u(xy, 64);
	}
	xr_string Id() const
	{
		u8 xy[64], digest[32];
		memcpy(xy, x, 32);
		memcpy(xy + 32, y, 32);
		Sha256(xy, 64, digest);
		return Hex(digest, 8);
	}
};

bool GenerateKey(SEcKey& key)
{
	BCRYPT_ALG_HANDLE alg = 0;
	BCRYPT_KEY_HANDLE handle = 0;
	u8 blob[sizeof(BCRYPT_ECCKEY_BLOB) + 96];
	ULONG size = 0;
	bool ok = BCRYPT_SUCCESS(::BCryptOpenAlgorithmProvider(&alg, BCRYPT_ECDSA_P256_ALGORITHM, 0, 0)) &&
		BCRYPT_SUCCESS(::BCryptGenerateKeyPair(alg, &handle, 256, 0)) &&
		BCRYPT_SUCCESS(::BCryptFinalizeKeyPair(handle, 0)) &&
		BCRYPT_SUCCESS(::BCryptExportKey(handle, 0, BCRYPT_ECCPRIVATE_BLOB, blob, sizeof(blob), &size, 0)) &&
		size == sizeof(blob);
	if (ok)
	{
		memcpy(key.x, blob + sizeof(BCRYPT_ECCKEY_BLOB), 32);
		memcpy(key.y, blob + sizeof(BCRYPT_ECCKEY_BLOB) + 32, 32);
		memcpy(key.d, blob + sizeof(BCRYPT_ECCKEY_BLOB) + 64, 32);
		key.has_private = true;
	}
	Wipe(blob, sizeof(blob));
	if (handle) ::BCryptDestroyKey(handle);
	if (alg) ::BCryptCloseAlgorithmProvider(alg, 0);
	return ok;
}

// ECDSA over SHA-256 of `data`: r || s, 64 bytes.
bool SignBytes(const SEcKey& key, const void* data, size_t size, u8 (&signature)[64])
{
	u8 digest[32];
	if (!Sha256(data, size, digest)) return false;
	u8 blob[sizeof(BCRYPT_ECCKEY_BLOB) + 96];
	BCRYPT_ECCKEY_BLOB* header = (BCRYPT_ECCKEY_BLOB*)blob;
	header->dwMagic = BCRYPT_ECDSA_PRIVATE_P256_MAGIC;
	header->cbKey = 32;
	memcpy(blob + sizeof(BCRYPT_ECCKEY_BLOB), key.x, 32);
	memcpy(blob + sizeof(BCRYPT_ECCKEY_BLOB) + 32, key.y, 32);
	memcpy(blob + sizeof(BCRYPT_ECCKEY_BLOB) + 64, key.d, 32);
	BCRYPT_ALG_HANDLE alg = 0;
	BCRYPT_KEY_HANDLE handle = 0;
	ULONG written = 0;
	bool ok = BCRYPT_SUCCESS(::BCryptOpenAlgorithmProvider(&alg, BCRYPT_ECDSA_P256_ALGORITHM, 0, 0)) &&
		BCRYPT_SUCCESS(::BCryptImportKeyPair(alg, 0, BCRYPT_ECCPRIVATE_BLOB, &handle, blob, sizeof(blob), 0)) &&
		BCRYPT_SUCCESS(::BCryptSignHash(handle, 0, digest, 32, signature, 64, &written, 0)) && written == 64;
	Wipe(blob, sizeof(blob));
	if (handle) ::BCryptDestroyKey(handle);
	if (alg) ::BCryptCloseAlgorithmProvider(alg, 0);
	return ok;
}

// ECDSA over SHA-256 of `data` by the public point x, y; CNG refuses a point off the curve.
bool VerifyBytes(const u8* x, const u8* y, const void* data, size_t size, const u8 (&signature)[64])
{
	u8 blob[sizeof(BCRYPT_ECCKEY_BLOB) + 64];
	BCRYPT_ECCKEY_BLOB* header = (BCRYPT_ECCKEY_BLOB*)blob;
	header->dwMagic = BCRYPT_ECDSA_PUBLIC_P256_MAGIC;
	header->cbKey = 32;
	memcpy(blob + sizeof(BCRYPT_ECCKEY_BLOB), x, 32);
	memcpy(blob + sizeof(BCRYPT_ECCKEY_BLOB) + 32, y, 32);
	u8 digest[32];
	if (!Sha256(data, size, digest)) return false;
	BCRYPT_ALG_HANDLE alg = 0;
	BCRYPT_KEY_HANDLE handle = 0;
	const bool ok = BCRYPT_SUCCESS(::BCryptOpenAlgorithmProvider(&alg, BCRYPT_ECDSA_P256_ALGORITHM, 0, 0)) &&
		BCRYPT_SUCCESS(::BCryptImportKeyPair(alg, 0, BCRYPT_ECCPUBLIC_BLOB, &handle, blob, sizeof(blob), 0)) &&
		BCRYPT_SUCCESS(::BCryptVerifySignature(handle, 0, digest, 32, (PUCHAR)signature, 64, 0));
	if (handle) ::BCryptDestroyKey(handle);
	if (alg) ::BCryptCloseAlgorithmProvider(alg, 0);
	return ok;
}

// The public point of the key file matches the private scalar: a signature by the scalar
// verifies under the point.
bool KeyConsistent(const SEcKey& key)
{
	u8 message[1] = { 0 };
	u8 signature[64];
	return SignBytes(key, message, 1, signature) && VerifyBytes(key.x, key.y, message, 1, signature);
}

//------------------------------------------------------------------------------
// files and folders
//------------------------------------------------------------------------------
bool ReadFileBytes(LPCSTR path, u64 limit, xr_string& out)
{
	out.clear();
	HANDLE f = ::CreateFileA(path, GENERIC_READ, FILE_SHARE_READ, 0, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0);
	if (INVALID_HANDLE_VALUE == f) return false;
	LARGE_INTEGER size;
	bool ok = ::GetFileSizeEx(f, &size) && (u64)size.QuadPart <= limit;
	if (ok)
	{
		out.resize((size_t)size.QuadPart);
		DWORD got = 0;
		ok = out.empty() || (::ReadFile(f, &out[0], (DWORD)out.size(), &got, 0) && got == out.size());
	}
	::CloseHandle(f);
	return ok;
}

bool WriteNewFile(LPCSTR path, const xr_string& text)
{
	HANDLE f = ::CreateFileA(path, GENERIC_WRITE, 0, 0, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, 0);
	if (INVALID_HANDLE_VALUE == f) return false;
	DWORD written = 0;
	const bool ok = ::WriteFile(f, text.data(), (DWORD)text.size(), &written, 0) && written == text.size();
	::CloseHandle(f);
	if (!ok) ::DeleteFileA(path);
	return ok;
}

void MakeFolders(LPCSTR file)
{
	xr_string dir = file;
	const size_t slash = dir.find_last_of("\\/");
	if (slash == xr_string::npos) return;
	dir.resize(slash);
	for (size_t i = 3; i <= dir.size(); ++i)
		if (i == dir.size() || dir[i] == '\\' || dir[i] == '/')
			::CreateDirectoryA(dir.substr(0, i).c_str(), 0);
}

xr_string EnvText(LPCSTR name)
{
	char buf[1024];
	const DWORD n = ::GetEnvironmentVariableA(name, buf, sizeof(buf));
	return (n && n < sizeof(buf)) ? xr_string(buf) : xr_string();
}

xr_string DocumentsFolder()
{
	PWSTR wide = 0;
	xr_string out;
	if (SUCCEEDED(::SHGetKnownFolderPath(FOLDERID_Documents, 0, 0, &wide)))
	{
		const int n = ::WideCharToMultiByte(CP_UTF8, 0, wide, -1, 0, 0, 0, 0);
		if (n > 1)
		{
			out.resize(n - 1);
			::WideCharToMultiByte(CP_UTF8, 0, wide, -1, &out[0], n, 0, 0);
		}
	}
	if (wide) ::CoTaskMemFree(wide);
	return out;
}

// Documents\XFined Editor holds the author's key and remembers where it is: a setting of this
// Windows user, never of a project. A -qa_github run is pointed elsewhere, so a test that makes
// keys never replaces the author's remembered one.
xr_string HomeFolder()
{
	static const bool qa = 0 != strstr(::GetCommandLineA(), "-qa_github");
	const xr_string home = qa ? EnvText("XFINED_CATALOG_HOME") : xr_string();
	return home.empty() ? DocumentsFolder() + "\\XFined Editor" : home;
}

xr_string SettingsPath() { return HomeFolder() + "\\catalog.ltx"; }

xr_string RememberedKeyPath()
{
	xr_string text;
	if (!ReadFileBytes(SettingsPath().c_str(), 64u << 10, text)) return xr_string();
	const size_t at = text.find("\nkey = ");
	if (at == xr_string::npos) return xr_string();
	const size_t end = text.find_first_of("\r\n", at + 7);
	return text.substr(at + 7, end == xr_string::npos ? xr_string::npos : end - at - 7);
}

void RememberKeyPath(LPCSTR path)
{
	const xr_string file = SettingsPath();
	MakeFolders(file.c_str());
	const xr_string text = xr_string("; XFined Editor: where this Windows user's Mod Browser author key file is\r\n[author]\r\nkey = ") +
		path + "\r\n";
	::DeleteFileA(file.c_str());
	WriteNewFile(file.c_str(), text);
}

//------------------------------------------------------------------------------
// the key file (MOD_CATALOG 4.1)
//------------------------------------------------------------------------------
struct SKeyFile
{
	xr_string	public_text;
	TBytes		private_raw;
	bool		protected_;
	u32			rounds;
	TBytes		salt, nonce, mac;

	SKeyFile() : protected_(false), rounds(0) {}
	~SKeyFile() { if (!private_raw.empty()) Wipe(private_raw.data(), private_raw.size()); }
};

xr_string Field(const xr_string& text, LPCSTR name)
{
	size_t at = 0;
	while (at < text.size())
	{
		size_t end = text.find('\n', at);
		if (end == xr_string::npos) end = text.size();
		xr_string line = text.substr(at, end - at);
		at = end + 1;
		const size_t eq = line.find('=');
		if (eq == xr_string::npos) continue;
		xr_string key = line.substr(0, eq), value = line.substr(eq + 1);
		while (!key.empty() && (key.back() == ' ' || key.back() == '\t')) key.pop_back();
		while (!value.empty() && (value.front() == ' ' || value.front() == '\t')) value.erase(0, 1);
		while (!value.empty() && (value.back() == ' ' || value.back() == '\r')) value.pop_back();
		if (key == name) return value;
	}
	return xr_string();
}

bool ParseKeyFile(const xr_string& text, SKeyFile& out, xr_string& err)
{
	if (0 != text.compare(0, 9, "xms-key 1"))		{ err = "not an XFined key file"; return false; }
	out.public_text = Field(text, "public");
	if (0 != out.public_text.compare(0, 5, "p256:"))	{ err = "the key file names no public key"; return false; }
	if (!UnB64u(Field(text, "private"), out.private_raw) || out.private_raw.size() != 32)
		{ err = "the key file holds no private key"; return false; }
	const xr_string kdf = Field(text, "kdf");
	out.protected_ = !kdf.empty();
	if (!out.protected_) return true;
	xr_vector<xr_string> parts;
	{
		xr_string part;
		for (size_t i = 0; i <= kdf.size(); ++i)
			if (i == kdf.size() || kdf[i] == ',') { while (!part.empty() && part[0] == ' ') part.erase(0, 1); parts.push_back(part); part.clear(); }
			else part += kdf[i];
	}
	const xr_string cipher = Field(text, "cipher");
	const size_t comma = cipher.find(',');
	if (parts.size() != 3 || parts[0] != "pbkdf2-sha256" || comma == xr_string::npos || cipher.substr(0, comma) != "hmac-sha256-ctr")
		{ err = "the key file is protected in a way this editor does not know"; return false; }
	out.rounds = (u32)strtoul(parts[1].c_str(), 0, 10);
	xr_string nonce = cipher.substr(comma + 1);
	while (!nonce.empty() && nonce[0] == ' ') nonce.erase(0, 1);
	if (!out.rounds || !UnB64u(parts[2], out.salt) || !UnB64u(nonce, out.nonce) || !UnB64u(Field(text, "mac"), out.mac) ||
		out.mac.size() != 32)
		{ err = "the key file's protection is damaged"; return false; }
	return true;
}

bool PublicPoint(const xr_string& text, SEcKey& key)
{
	TBytes raw;
	if (0 != text.compare(0, 5, "p256:") || !UnB64u(text.substr(5), raw) || raw.size() != 64) return false;
	memcpy(key.x, raw.data(), 32);
	memcpy(key.y, raw.data() + 32, 32);
	return true;
}

bool OpenKeyFile(const SKeyFile& file, const xr_string& password, SEcKey& key, xr_string& err)
{
	if (!PublicPoint(file.public_text, key)) { err = "the key file's public key is damaged"; return false; }
	TBytes scalar = file.private_raw;
	if (file.protected_)
	{
		u8 derived[64];
		if (!Pbkdf2(password, file.salt, file.rounds, derived)) { err = "can't derive the key file's key"; return false; }
		u8 tag[32];
		const bool mac_ok = Hmac(derived + 32, 32, file.nonce.data(), file.nonce.size(), scalar.data(), scalar.size(), tag) &&
			0 == memcmp(tag, file.mac.data(), 32);
		if (!mac_ok) { Wipe(derived, sizeof(derived)); err = "wrong password"; return false; }
		Stream(derived, file.nonce, scalar.data(), scalar.size());
		Wipe(derived, sizeof(derived));
	}
	memcpy(key.d, scalar.data(), 32);
	Wipe(scalar.data(), scalar.size());
	key.has_private = true;
	if (!KeyConsistent(key)) { err = "the key file's private key does not match its public key"; return false; }
	return true;
}

xr_string KeyFileText(const SEcKey& key, const xr_string& password)
{
	xr_string text = "xms-key 1\npublic  = " + key.PublicText() + "\n";
	if (password.empty())
		return text + "private = " + B64u(key.d, 32) + "\n";
	u8 salt[16], nonce[16], derived[64], encrypted[32], tag[32];
	Random(salt, 16);
	Random(nonce, 16);
	TBytes salt_v, nonce_v;
	salt_v.assign(salt, salt + 16);
	nonce_v.assign(nonce, nonce + 16);
	Pbkdf2(password, salt_v, kKdfRounds, derived);
	memcpy(encrypted, key.d, 32);
	Stream(derived, nonce_v, encrypted, 32);
	Hmac(derived + 32, 32, nonce, 16, encrypted, 32, tag);
	Wipe(derived, sizeof(derived));
	text += NqUtil::Format("kdf     = pbkdf2-sha256, %u, ", kKdfRounds) + B64u(salt, 16) + "\n";
	text += "cipher  = hmac-sha256-ctr, " + B64u(nonce, 16) + "\n";
	text += "private = " + B64u(encrypted, 32) + "\n";
	text += "mac     = " + B64u(tag, 32) + "\n";
	return text;
}

// the key opened this session; locked again when the editor closes
std::mutex	s_KeyLock;
SEcKey		s_Key;
xr_string	s_KeyPath;		// the file s_Key came from
// bumped by everything that changes the key or its file: a dialog's cached state is stale
std::atomic<u32>	s_KeyGeneration(1);

xr_string CurrentKeyPath()
{
	const xr_string remembered = RememberedKeyPath();
	return remembered.empty() ? EditorCatalog::DefaultKeyPath() : remembered;
}

// A key file inside the open project would travel with it - into its git repository, into
// every build a player downloads. The build leaves *.xkey out anyway; this says it first.
bool InsideProject(const xr_string& path, xr_string& err)
{
	if (!EditorProject::Active()) return false;
	char full[MAX_PATH * 3] = {}, root[MAX_PATH * 3] = {};
	if (!::GetFullPathNameA(path.c_str(), sizeof(full), full, 0) || !::GetFullPathNameA(EditorProject::Root(), sizeof(root), root, 0))
		return false;
	size_t n = xr_strlen(root);
	while (n && (root[n - 1] == '\\' || root[n - 1] == '/')) root[--n] = 0;
	if (0 != _strnicmp(full, root, n) || (full[n] != '\\' && full[n] != '/')) return false;
	err = "a key file never goes inside a project - it would ship with the module; keep it in Documents or on a stick";
	return true;
}

//------------------------------------------------------------------------------
// the card
//------------------------------------------------------------------------------
xr_string QuoteText(const xr_string& text)
{
	xr_string out = "\"";
	for (size_t i = 0; i < text.size(); ++i)
	{
		const char c = text[i];
		if (c == '"' || c == '\\')	{ out += '\\'; out += c; }
		else if (c == '\n')			out += "\\n";
		else if (c != '\r')			out += c;
	}
	return out + "\"";
}

// <string id="id"><text>...</text> in the project's string tables of `language`, UTF-8
bool LookUpString(LPCSTR project_root, LPCSTR language, const xr_string& id, xr_string& out)
{
	string_path mask;
	sprintf_s(mask, "%s\\gamedata\\configs\\text\\%s\\*.xml", project_root, language);
	WIN32_FIND_DATAA fd;
	HANDLE h = ::FindFirstFileA(mask, &fd);
	if (INVALID_HANDLE_VALUE == h) return false;
	bool found = false;
	const xr_string open = "id=\"" + id + "\"";
	do
	{
		string_path file;
		sprintf_s(file, "%s\\gamedata\\configs\\text\\%s\\%s", project_root, language, fd.cFileName);
		xr_string text;
		if (!ReadFileBytes(file, 16u << 20, text)) continue;
		const size_t at = text.find(open);
		if (at == xr_string::npos) continue;
		const size_t begin = text.find("<text>", at);
		const size_t end = begin == xr_string::npos ? begin : text.find("</text>", begin);
		const size_t next = text.find("<string", at + open.size());
		if (end == xr_string::npos || (next != xr_string::npos && begin > next)) continue;
		out = NqUtil::GameTextToUtf8(text.data() + begin + 6, u32(end - begin - 6));
		// the string tables' own escapes: \n is a line break there too
		xr_string plain;
		for (size_t i = 0; i < out.size(); ++i)
			if (out[i] == '\\' && i + 1 < out.size() && out[i + 1] == 'n') { plain += '\n'; ++i; }
			else plain += out[i];
		out = plain;
		found = true;
	} while (!found && ::FindNextFileA(h, &fd));
	::FindClose(h);
	return found;
}

bool LooksLikeId(const xr_string& text)
{
	if (text.empty() || text.size() > 128) return false;
	for (size_t i = 0; i < text.size(); ++i)
		if (text[i] == ' ' || text[i] == '\t' || text[i] == '\n' || (unsigned char)text[i] >= 0x80) return false;
	return true;
}

// A display text of the manifest: the Russian (or the only) text and the English one, if any.
void Resolve(LPCSTR project_root, const xr_string& value, xr_string& rus, xr_string& eng)
{
	rus = value;
	eng.clear();
	if (!LooksLikeId(value)) return;
	xr_string text;
	if (LookUpString(project_root, "rus", value, text)) rus = text;
	if (LookUpString(project_root, "eng", value, text)) eng = text;
	if (rus == value && !eng.empty()) rus = eng;
}

void AppendCardText(xr_string& out, LPCSTR key, LPCSTR project_root, const xr_string& value)
{
	if (value.empty()) return;
	xr_string rus, eng;
	Resolve(project_root, value, rus, eng);
	out += NqUtil::Format("%-11s = ", key) + QuoteText(rus) + "\r\n";
	if (!eng.empty() && eng != rus)
		out += NqUtil::Format("%-11s = ", (xr_string(key) + "@eng").c_str()) + QuoteText(eng) + "\r\n";
}
} // namespace

//==============================================================================
// the key
//==============================================================================
xr_string EditorCatalog::DefaultKeyPath() { return HomeFolder() + "\\Keys\\author.xkey"; }

EditorCatalog::SKeyState EditorCatalog::KeyStateForUI()
{
	static SKeyState cached;
	static u32 generation = 0, read = 0;
	if (generation != s_KeyGeneration || !read || ::GetTickCount() - read > 1000)
	{
		cached = KeyState();
		generation = s_KeyGeneration;
		read = ::GetTickCount();
	}
	return cached;
}

EditorCatalog::SKeyState EditorCatalog::KeyState()
{
	SKeyState state;
	state.path = CurrentKeyPath();
	xr_string text;
	state.exists = ReadFileBytes(state.path.c_str(), kKeyFileLimit, text);
	if (!state.exists) return state;
	SKeyFile file;
	if (!ParseKeyFile(text, file, state.error)) return state;
	SEcKey point;
	if (!PublicPoint(file.public_text, point)) { state.error = "the key file's public key is damaged"; return state; }
	state.public_key = file.public_text;
	state.key_id = point.Id();
	std::lock_guard<std::mutex> guard(s_KeyLock);
	state.locked = file.protected_ && !(s_Key.has_private && s_KeyPath == state.path && s_Key.PublicText() == file.public_text);
	return state;
}

bool EditorCatalog::CreateKey(LPCSTR path, LPCSTR password, xr_string& err)
{
	const xr_string target = (path && path[0]) ? xr_string(path) : DefaultKeyPath();
	if (InsideProject(target, err)) return false;
	if (::GetFileAttributesA(target.c_str()) != INVALID_FILE_ATTRIBUTES)
		{ err = target + " exists: a key file is never overwritten"; return false; }
	SEcKey key;
	if (!GenerateKey(key)) { err = "Windows could not make a P-256 key"; return false; }
	MakeFolders(target.c_str());
	if (!WriteNewFile(target.c_str(), KeyFileText(key, password ? password : "")))
		{ err = "can't write " + target; return false; }
	RememberKeyPath(target.c_str());
	std::lock_guard<std::mutex> guard(s_KeyLock);
	s_Key = key;
	s_KeyPath = target;
	++s_KeyGeneration;
	Msg("* [catalog] author key %s created in %s", key.Id().c_str(), target.c_str());
	return true;
}

bool EditorCatalog::UseKeyFile(LPCSTR path, xr_string& err)
{
	xr_string text;
	if (!path || !ReadFileBytes(path, kKeyFileLimit, text)) { err = xr_string("can't read ") + (path ? path : ""); return false; }
	if (InsideProject(path, err)) return false;
	SKeyFile file;
	if (!ParseKeyFile(text, file, err)) return false;
	RememberKeyPath(path);
	++s_KeyGeneration;
	if (!file.protected_)
	{
		SEcKey key;
		if (!OpenKeyFile(file, "", key, err)) return false;
		std::lock_guard<std::mutex> guard(s_KeyLock);
		s_Key = key;
		s_KeyPath = path;
	}
	Msg("* [catalog] author key file: %s", path);
	return true;
}

bool EditorCatalog::Unlock(LPCSTR password, xr_string& err)
{
	const xr_string path = CurrentKeyPath();
	xr_string text;
	if (!ReadFileBytes(path.c_str(), kKeyFileLimit, text)) { err = "no author key file at " + path; return false; }
	SKeyFile file;
	if (!ParseKeyFile(text, file, err)) return false;
	SEcKey key;
	if (!OpenKeyFile(file, password ? password : "", key, err)) return false;
	std::lock_guard<std::mutex> guard(s_KeyLock);
	s_Key = key;
	s_KeyPath = path;
	++s_KeyGeneration;
	return true;
}

bool EditorCatalog::Sign(const xr_string& body, xr_string& out, xr_string& err)
{
	// an unprotected key file opens on its own
	const SKeyState state = KeyState();
	if (!state.exists)	{ err = "no author key: Mod > Submit to Mod Browser makes one"; return false; }
	if (!state.error.empty())	{ err = state.error; return false; }
	if (state.locked)	{ err = "the author key is locked: give its password"; return false; }
	{
		std::lock_guard<std::mutex> guard(s_KeyLock);
		if (!(s_Key.has_private && s_KeyPath == state.path))
		{
			xr_string text;
			SKeyFile file;
			SEcKey key;
			if (!ReadFileBytes(state.path.c_str(), kKeyFileLimit, text) || !ParseKeyFile(text, file, err) ||
				!OpenKeyFile(file, "", key, err))
				return false;
			s_Key = key;
			s_KeyPath = state.path;
		}
	}
	const bool crlf = body.find("\r\n") != xr_string::npos;
	const char* nl = crlf ? "\r\n" : "\n";
	xr_string signed_body = body;
	if (signed_body.empty() || signed_body.back() != '\n') signed_body += nl;
	u8 signature[64];
	std::lock_guard<std::mutex> guard(s_KeyLock);
	if (!SignBytes(s_Key, signed_body.data(), signed_body.size(), signature)) { err = "Windows could not sign"; return false; }
	out = signed_body + "[signature]" + nl + "key = " + s_Key.PublicText() + nl + "sig = " + s_Key.Id() + ":" + B64u(signature, 64) + nl;
	return true;
}

// MOD_CATALOG 3.1: the message is every byte before a line that is exactly [signature]; after
// it only the key and sig lines and blank ones.
bool EditorCatalog::Verify(const xr_string& text, const xr_string& public_key, xr_string& signer, xr_string& err)
{
	signer.clear();
	size_t header = xr_string::npos;
	for (size_t at = 0; at < text.size();)
	{
		size_t end = text.find('\n', at);
		if (end == xr_string::npos) end = text.size();
		size_t line_end = end;
		if (line_end > at && text[line_end - 1] == '\r') --line_end;
		// the game and the catalog's tools do not take a header on the very first line either
		if (at && line_end - at == 11 && 0 == text.compare(at, 11, "[signature]"))
		{
			if (header != xr_string::npos) { err = "the text is signed twice"; return false; }
			header = at;
		}
		at = end + 1;
	}
	if (header == xr_string::npos) { err = "not signed"; return false; }

	xr_string key_text, sig_text;
	for (size_t at = text.find('\n', header); at != xr_string::npos && at + 1 < text.size();)
	{
		const size_t begin = at + 1;
		at = text.find('\n', begin);
		xr_string line = text.substr(begin, (at == xr_string::npos ? text.size() : at) - begin);
		while (!line.empty() && (line.back() == '\r' || line.back() == ' ' || line.back() == '\t')) line.pop_back();
		if (line.empty()) continue;
		const size_t eq = line.find('=');
		xr_string name = eq == xr_string::npos ? line : line.substr(0, eq);
		xr_string value = eq == xr_string::npos ? xr_string() : line.substr(eq + 1);
		while (!name.empty() && (name.back() == ' ' || name.back() == '\t')) name.pop_back();
		while (!value.empty() && (value[0] == ' ' || value[0] == '\t')) value.erase(0, 1);
		if (name == "key" && key_text.empty())		key_text = value;
		else if (name == "sig" && sig_text.empty())	sig_text = value;
		else { err = "the signature section holds more than a key and a signature"; return false; }
	}
	const size_t colon = sig_text.find(':');
	if (colon == xr_string::npos) { err = "the signature is damaged"; return false; }
	signer = sig_text.substr(0, colon);

	SEcKey expected;
	TBytes raw;
	if (!PublicPoint(public_key, expected)) { err = "the expected key is damaged"; return false; }
	if (signer != expected.Id()) { err = "signed with another key (" + signer + ")"; return false; }
	if (!UnB64u(sig_text.substr(colon + 1), raw) || raw.size() != 64) { err = "the signature is damaged"; return false; }
	u8 signature[64];
	memcpy(signature, raw.data(), 64);
	if (!VerifyBytes(expected.x, expected.y, text.data(), header, signature))
		{ err = "the signature does not match the text: it was changed after signing"; return false; }
	return true;
}

void EditorCatalog::DrawKeyLine(LPCSTR id)
{
	static char password[128] = {};
	static xr_string problem;
	const SKeyState state = KeyStateForUI();
	ImGui::PushID(id);
	if (!state.exists)
		ImGui::TextColored(ImVec4(1.f, 0.8f, 0.3f, 1.f), "Not signed: no author key (Mod > Submit to Mod Browser sets one up).");
	else if (!state.error.empty())
		ImGui::TextColored(ImVec4(1.f, 0.4f, 0.4f, 1.f), "Not signed: %s", state.error.c_str());
	else if (state.locked)
	{
		ImGui::TextUnformatted("Author key password:");
		ImGui::SameLine();
		ImGui::SetNextItemWidth(200);
		const bool enter = ImGui::InputText("##key_password", password, sizeof(password),
			ImGuiInputTextFlags_Password | ImGuiInputTextFlags_EnterReturnsTrue);
		ImGui::SameLine();
		if (ImGui::Button("Unlock") || enter)
		{
			xr_string err;
			problem = Unlock(password, err) ? xr_string() : err;
			Wipe(password, sizeof(password));
		}
		if (!problem.empty())
			ImGui::TextColored(ImVec4(1.f, 0.4f, 0.4f, 1.f), "%s", problem.c_str());
	}
	else
		ImGui::TextColored(ImVec4(0.5f, 0.85f, 0.5f, 1.f), "Signed with the author key %s.", state.key_id.c_str());
	ImGui::PopID();
}

void EditorCatalog::Shutdown()
{
	std::lock_guard<std::mutex> guard(s_KeyLock);
	s_Key = SEcKey();
	s_KeyPath.clear();
	++s_KeyGeneration;
}

//==============================================================================
// the card
//==============================================================================
xr_string EditorCatalog::CardSection(LPCSTR project_root, const EditorMod::SManifest& m, const SCardFile& manifest,
									 const SCardFile* thumbnail)
{
	xr_string out = "\r\n[card]\r\n";
	AppendCardText(out, "name", project_root, m.name.empty() ? m.id : m.name);
	AppendCardText(out, "author", project_root, m.author);
	AppendCardText(out, "description", project_root, m.description);
	if (!m.website.empty())
		out += NqUtil::Format("%-11s = %s\r\n", "website", m.website.c_str());

	// the mode: the plain game, one the module brings, or one somebody else's
	xr_string mode = "plain", title;
	if (!m.mode.empty() && m.mode != "*")
	{
		bool own = false;
		for (u32 i = 0; i < m.provides_modes.size(); ++i)
			if (m.provides_modes[i].id == m.mode) { own = true; title = m.provides_modes[i].title; }
		mode = (own ? "new " : "mode ") + m.mode;
		if (title.empty()) title = m.mode;
	}
	out += NqUtil::Format("%-11s = %s\r\n", "mode", mode.c_str());
	AppendCardText(out, "mode_title", project_root, title);

	if (!m.tags.empty())
	{
		xr_string tags;
		for (u32 i = 0; i < m.tags.size(); ++i)
			tags += (i ? ", " : "") + m.tags[i];
		out += NqUtil::Format("%-11s = %s\r\n", "tags", tags.c_str());
	}
	if (thumbnail)
		out += NqUtil::Format("%-11s = %s, %llu, %s\r\n", "thumbnail", thumbnail->path.c_str(), thumbnail->size, thumbnail->sha256.c_str());
	out += NqUtil::Format("%-11s = %s, %llu, %s\r\n", "manifest", manifest.path.c_str(), manifest.size, manifest.sha256.c_str());
	return out;
}

const xr_vector<xr_string>& EditorCatalog::TagVocabulary()
{
	static const char* words[] = { "story", "quests", "gameplay", "weapons", "equipment", "npcs", "mutants",
		"locations", "graphics", "audio", "interface", "fixes" };
	static xr_vector<xr_string> tags;
	if (tags.empty())
		for (u32 i = 0; i < sizeof(words) / sizeof(words[0]); ++i) tags.push_back(words[i]);
	return tags;
}

bool EditorCatalog::KnownTag(LPCSTR tag)
{
	const xr_vector<xr_string>& tags = TagVocabulary();
	for (u32 i = 0; i < tags.size(); ++i)
		if (tags[i] == tag) return true;
	return false;
}

LPCSTR EditorCatalog::TagLabel(LPCSTR tag)
{
	static const std::pair<LPCSTR, LPCSTR> labels[] = { {"story", "Story"}, {"quests", "Quests"}, {"gameplay", "Gameplay"},
		{"weapons", "Weapons"}, {"equipment", "Equipment"}, {"npcs", "Characters"}, {"mutants", "Mutants"},
		{"locations", "Locations"}, {"graphics", "Graphics"}, {"audio", "Sound"}, {"interface", "Interface"}, {"fixes", "Fixes"} };
	for (const auto& label : labels)
		if (0 == xr_strcmp(tag, label.first)) return label.second;
	return tag;
}

//==============================================================================
// the website
//==============================================================================
namespace
{
bool PathSegment(LPCSTR& p, bool (*allowed)(char))
{
	const char* start = p;
	while (*p && *p != '/' && *p != '?' && *p != '#' && allowed(*p)) ++p;
	return p != start;
}
bool SlugChar(char c) { return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_' || c == '-'; }
bool DigitChar(char c) { return c >= '0' && c <= '9'; }

bool Starts(LPCSTR& p, LPCSTR prefix)
{
	const size_t n = xr_strlen(prefix);
	if (0 != _strnicmp(p, prefix, n)) return false;
	p += n;
	return true;
}

// what may follow an AP-PRO page: an optional slash, then nothing or a query / fragment
bool PageEnd(LPCSTR p)
{
	if (*p == '/') ++p;
	return !*p || *p == '?' || *p == '#';
}
} // namespace

bool EditorCatalog::WebsiteIsModulePage(LPCSTR url)
{
	if (!url || !EditorMod::ValidateWebsite(url)) return false;
	LPCSTR p = url;
	if (!Starts(p, "https://")) return false;
	Starts(p, "www.");
	if (Starts(p, "ap-pro.ru/"))
	{
		// /stuff/<section>/<slug>-r<number>/ or /forums/topic/<number>-<slug>/
		if (Starts(p, "stuff/"))
		{
			if (!PathSegment(p, SlugChar) || *p != '/') return false;
			++p;
			LPCSTR slug = p;
			if (!PathSegment(p, SlugChar)) return false;
			xr_string last;
			last.assign(slug, p);
			const size_t r = last.rfind("-r");
			if (r == xr_string::npos || r + 2 >= last.size()) return false;
			for (size_t i = r + 2; i < last.size(); ++i)
				if (!DigitChar(last[i])) return false;
			return PageEnd(p);
		}
		if (Starts(p, "forums/topic/"))
		{
			LPCSTR number = p;
			while (DigitChar(*p)) ++p;
			return p != number && *p == '-' && PathSegment(++p, SlugChar) && PageEnd(p);
		}
		return false;
	}
	// a ModDB page may go on into its own sub-pages (/downloads, /images), not into a query
	if (Starts(p, "moddb.com/"))
	{
		if (Starts(p, "mods/") || Starts(p, "addons/"))
			return PathSegment(p, SlugChar) && (!*p || *p == '/');
		if (Starts(p, "games/"))
			return PathSegment(p, SlugChar) && Starts(p, "/addons/") && PathSegment(p, SlugChar) && (!*p || *p == '/');
	}
	return false;
}

//==============================================================================
// content (MOD_CATALOG 8)
//==============================================================================
bool EditorCatalog::PathAllowed(LPCSTR path, xr_string& why)
{
	const xr_string p = path ? path : "";
	if (p.empty() || p[0] == '/' || p.find('\\') != xr_string::npos || p.find(':') != xr_string::npos)
		{ why = "unsafe path"; return false; }
	size_t start = 0;
	for (;;)
	{
		const size_t end = p.find('/', start);
		const xr_string part = p.substr(start, end == xr_string::npos ? xr_string::npos : end - start);
		if (part.empty() || part == "." || part == "..") { why = "unsafe path"; return false; }
		if (part[0] == '.') { why = "a dot-named file or folder"; return false; }
		if (end == xr_string::npos) break;
		start = end + 1;
	}
	const size_t slash = p.rfind('/');
	const xr_string name = p.substr(slash == xr_string::npos ? 0 : slash + 1);
	const size_t dot = name.rfind('.');
	if (dot == xr_string::npos) return true;
	xr_string extension = name.substr(dot + 1);
	for (size_t i = 0; i < extension.size(); ++i)
		if (extension[i] >= 'A' && extension[i] <= 'Z') extension[i] = char(extension[i] - 'A' + 'a');
	static const char* allowed[] = { "ltx", "ltxp", "xml", "xmlp", "script", "txt", "md", "json", "csv", "ini", "cfg",
		"seq", "nqasset", "behasset", "pdaasset", "xspawn", "xcform", "dds", "thm", "ogg", "ogm", "ogf", "omf", "skl",
		"skls", "anm", "ppe", "xr", "ps", "vs", "gs", "hs", "ds", "cs", "h", "hlsl", "s", "spawn", "graph", "ai", "gct",
		"cform", "details", "env_mod", "fog_vol", "game", "geom", "geomx", "hom", "ps_static", "snd_env", "snd_static",
		"som", "sectors", "wallmarks", "lanims" };
	for (u32 i = 0; i < sizeof(allowed) / sizeof(allowed[0]); ++i)
		if (extension == allowed[i]) return true;
	why = "." + extension + " files are not allowed in a Mod Browser module";
	return false;
}

bool EditorCatalog::BytesAllowed(const void* data, size_t size, xr_string& why)
{
	const u8* b = (const u8*)data;
	if (size >= 0x40 && b[0] == 'M' && b[1] == 'Z')
	{
		u32 offset = 0;
		memcpy(&offset, b + 0x3C, 4);
		if (offset + 4 <= size && 0 == memcmp(b + offset, "PE\0\0", 4)) { why = "a Windows program or library"; return false; }
	}
	struct SMagic { const char* bytes; u32 size; const char* what; };
	static const SMagic magics[] = {
		{"\x7F" "ELF", 4, "a Linux program"}, {"\xFE\xED\xFA\xCE", 4, "a macOS program"}, {"\xFE\xED\xFA\xCF", 4, "a macOS program"},
		{"\xCE\xFA\xED\xFE", 4, "a macOS program"}, {"\xCF\xFA\xED\xFE", 4, "a macOS program"},
		{"\xCA\xFE\xBA\xBE", 4, "a Java class or a universal binary"}, {"PK\x03\x04", 4, "a ZIP archive"},
		{"PK\x05\x06", 4, "a ZIP archive"}, {"Rar!\x1A\x07", 6, "a RAR archive"}, {"7z\xBC\xAF\x27\x1C", 6, "a 7-Zip archive"},
		{"MSCF", 4, "a CAB archive"}, {"\x1F\x8B", 2, "a GZIP archive"}, {"\xFD" "7zXZ", 5, "an XZ archive"},
		{"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1", 8, "an OLE container"}, {"L\0\0\0\x01\x14\x02\0", 8, "a Windows shortcut"},
		{"\0asm", 4, "WebAssembly"}, {"\x1BLua", 4, "Lua bytecode"}, {"\x1BLJ", 3, "LuaJIT bytecode"} };
	for (u32 i = 0; i < sizeof(magics) / sizeof(magics[0]); ++i)
		if (size >= magics[i].size && 0 == memcmp(b, magics[i].bytes, magics[i].size)) { why = magics[i].what; return false; }
	return true;
}

LPCSTR EditorCatalog::LevelName(ELevel level)
{
	switch (level)
	{
	case lvNote:	return "note";
	case lvReview:	return "review";
	case lvBlock:	return "block";
	}
	return "ok";
}

namespace
{
bool IsScript(LPCSTR path)
{
	const size_t n = xr_strlen(path);
	return n > 7 && 0 == _stricmp(path + n - 7, ".script");
}
} // namespace

size_t EditorCatalog::ContentHead(LPCSTR path, size_t size)
{
	// a script is read whole; anything else by what it starts with - a PE header sits within
	// the first kilobytes of any real program
	const size_t limit = IsScript(path) ? (16u << 20) : (64u << 10);
	return size < limit ? size : limit;
}

size_t EditorCatalog::ContentNeed(const void* head, size_t got, size_t size)
{
	// a program's PE header is found where its MZ header says, which may lie past the head
	const u8* b = (const u8*)head;
	if (got < 0x40 || b[0] != 'M' || b[1] != 'Z') return got;
	u32 offset = 0;
	memcpy(&offset, b + 0x3C, 4);
	const u64 need = u64(offset) + 4;
	return need > got && need <= size ? size_t(need) : got;
}

void EditorCatalog::CheckContent(LPCSTR path, const void* data, size_t size, xr_vector<SFinding>& out)
{
	xr_string why;
	if (!PathAllowed(path, why))
		{ out.push_back(SFinding(lvBlock, "content", xr_string(path) + ": " + why)); return; }
	if (!BytesAllowed(data, size, why))
		{ out.push_back(SFinding(lvBlock, "content", xr_string(path) + ": " + why)); return; }
	if (IsScript(path))
		ScriptReport(path, (const char*)data, size, out);
}

//==============================================================================
// the script report (MOD_CATALOG 10.3) - the same tokenizer and rules as the
// catalog's own tools/catalog.py, so the author meets the findings the pull
// request would meet
//==============================================================================
namespace
{
enum ETokenKind { tkName, tkString, tkNumber, tkOp, tkOther };

struct SToken
{
	ETokenKind	kind;
	xr_string	text;
	u32			line;
};

bool NameStart(char c)	{ return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_'; }
bool NameChar(char c)	{ return NameStart(c) || (c >= '0' && c <= '9'); }
bool Digit(char c)		{ return c >= '0' && c <= '9'; }

// [=*[ at `p`: the level of the long bracket, or -1
int LongOpen(const char* p, const char* end)
{
	if (p >= end || *p != '[') return -1;
	int level = 0;
	for (++p; p < end && *p == '='; ++p) ++level;
	return (p < end && *p == '[') ? level : -1;
}

// the long bracket of `level` opened at `p` closes: `stop` is past its ]=*]
bool LongClose(const char* p, const char* end, int level, const char*& stop)
{
	for (p += level + 2; p < end; ++p)
	{
		if (*p != ']') continue;
		const char* q = p + 1;
		int n = 0;
		while (q < end && *q == '=') { ++q; ++n; }
		if (n == level && q < end && *q == ']') { stop = q + 1; return true; }
	}
	return false;
}

void Tokenize(const char* text, size_t size, xr_vector<SToken>& out)
{
	const char* p = text;
	const char* const end = text + size;
	u32 line = 1;
	const auto count_lines = [&line](const char* a, const char* b) { for (; a < b; ++a) line += *a == '\n'; };
	while (p < end)
	{
		const char c = *p;
		if (c == ' ' || c == '\t' || c == '\r' || c == '\n' || c == '\f' || c == '\v') { line += c == '\n'; ++p; continue; }
		if (c == '-' && p + 1 < end && p[1] == '-')
		{
			const int level = LongOpen(p + 2, end);
			const char* stop = p;
			if (level >= 0 && LongClose(p + 2, end, level, stop)) { count_lines(p, stop); p = stop; continue; }
			// an unclosed long comment is an ordinary line comment, as the catalog reads it
			while (p < end && *p != '\n') ++p;
			continue;
		}
		SToken t;
		t.line = line;
		const int level = LongOpen(p, end);
		const char* stop = p;
		if (level >= 0 && LongClose(p, end, level, stop))
		{
			t.kind = tkString;
			t.text.assign(p, stop);
			count_lines(p, stop);
			p = stop;
			out.push_back(t);
			continue;
		}
		if (c == '"' || c == '\'')
		{
			const char* q = p + 1;
			while (q < end && *q != c && *q != '\n')
				q += (*q == '\\' && q + 1 < end) ? 2 : 1;
			if (q < end && *q == c)
			{
				t.kind = tkString;
				t.text.assign(p, q + 1);
				count_lines(p, q + 1);
				p = q + 1;
				out.push_back(t);
				continue;
			}
			// an unterminated quote is a stray character
			t.kind = tkOther;
			t.text.assign(p, p + 1);
			++p;
			out.push_back(t);
			continue;
		}
		if (Digit(c) || (c == '.' && p + 1 < end && Digit(p[1])))
		{
			const char* q = p;
			const bool hex = c == '0' && q + 2 < end && (q[1] == 'x' || q[1] == 'X') && (isxdigit((unsigned char)q[2]) || q[2] == '.');
			const auto exponent = [&q, end](char lower, char upper)
			{
				// only a complete exponent belongs to the number
				const char* e = q;
				if (e >= end || (*e != lower && *e != upper)) return;
				++e;
				if (e < end && (*e == '+' || *e == '-')) ++e;
				if (e >= end || !Digit(*e)) return;
				while (e < end && Digit(*e)) ++e;
				q = e;
			};
			if (hex)
			{
				q += 2;
				while (q < end && (isxdigit((unsigned char)*q) || *q == '.')) ++q;
				exponent('p', 'P');
			}
			else
			{
				while (q < end && Digit(*q)) ++q;
				if (q < end && *q == '.') ++q;
				while (q < end && Digit(*q)) ++q;
				exponent('e', 'E');
			}
			t.kind = tkNumber;
			t.text.assign(p, q);
			p = q;
			out.push_back(t);
			continue;
		}
		if (NameStart(c))
		{
			const char* q = p;
			while (q < end && NameChar(*q)) ++q;
			t.kind = tkName;
			t.text.assign(p, q);
			p = q;
			out.push_back(t);
			continue;
		}
		static const char* longer[] = { "...", "..", "==", "~=", "<=", ">=", "::" };
		size_t n = 0;
		for (u32 i = 0; i < sizeof(longer) / sizeof(longer[0]) && !n; ++i)
		{
			const size_t k = xr_strlen(longer[i]);
			if (size_t(end - p) >= k && 0 == strncmp(p, longer[i], k)) n = k;
		}
		if (!n && strchr("-+*/%^#&~|<>=(){}[];:,.", c) && c) n = 1;
		t.kind = n ? tkOp : tkOther;
		t.text.assign(p, p + (n ? n : 1));
		p += n ? n : 1;
		out.push_back(t);
	}
}

// a long string of base64-ish characters only
bool EncodedBlob(const xr_string& s)
{
	size_t a = 0, b = s.size();
	while (a < b && s[a] && strchr("\"'[=", s[a])) ++a;
	while (b > a && s[b - 1] && strchr("\"']=", s[b - 1])) --b;
	if (a == b) return false;
	for (size_t i = a; i < b; ++i)
	{
		const char c = s[i];
		if (!(NameChar(c) || c == '+' || c == '/' || c == '=' || c == '\\' || c == '-' || c == ' ' || c == '\t' ||
			  c == '\r' || c == '\n'))
			return false;
	}
	return true;
}
} // namespace

void EditorCatalog::ScriptReport(LPCSTR path, const char* text, size_t size, xr_vector<SFinding>& out)
{
	xr_vector<SToken> tokens;
	Tokenize(text, size, tokens);
	struct SPattern { const char* a; const char* b; const char* what; };
	static const SPattern blocking[] = { {"ffi", 0, "ffi"}, {"os", "execute", "os.execute"}, {"io", "popen", "io.popen"},
		{"package", "loadlib", "package.loadlib"}, {"string", "dump", "string.dump"}, {"lfs", "link", "lfs.link"},
		{"StartDedicatedServer", 0, "StartDedicatedServer"} };
	static const SPattern review[] = { {"setfenv", 0, "setfenv"}, {"getfenv", 0, "getfenv"}, {"debug", 0, "debug.*"},
		{"save_as", 0, "ini_file:save_as"}, {"w_open", 0, "getFS():w_open"}, {"file_delete", 0, "getFS():file_delete"},
		{"file_rename", 0, "getFS():file_rename"}, {"file_copy", 0, "getFS():file_copy"}, {"dir_delete", 0, "getFS():dir_delete"} };
	const auto add = [&out, path](ELevel level, u32 line, const xr_string& what)
		{ out.push_back(SFinding(level, "script", NqUtil::Format("%s:%u ", path, line) + what)); };
	const auto text_at = [&tokens](size_t i) -> const xr_string& { static const xr_string none; return i < tokens.size() ? tokens[i].text : none; };

	for (size_t i = 0; i < tokens.size(); ++i)
	{
		if (tokens[i].kind != tkName) continue;
		if (i > 0 && (tokens[i - 1].text == "." || tokens[i - 1].text == ":")) continue;
		// the dotted name starting here: a, a.b, a:b
		xr_vector<xr_string> parts(1, tokens[i].text);
		size_t after = i + 1;
		while (after + 1 < tokens.size() && (tokens[after].text == "." || tokens[after].text == ":") && tokens[after + 1].kind == tkName)
		{
			parts.push_back(tokens[after + 1].text);
			after += 2;
		}
		const u32 line = tokens[i].line;
		for (const SPattern& pattern : blocking)
			if (parts[0] == pattern.a && (!pattern.b || (parts.size() > 1 && parts[1] == pattern.b)))
				add(lvBlock, line, xr_string("uses ") + pattern.what);
		if (parts[0] == "require" && text_at(after) == "(" && after + 1 < tokens.size() && tokens[after + 1].kind == tkString &&
			tokens[after + 1].text.find("ffi") != xr_string::npos)
			add(lvBlock, line, "loads ffi");
		if ((parts[0] == "load" || parts[0] == "loadstring" || parts[0] == "require" || parts[0] == "dofile" || parts[0] == "loadfile") &&
			text_at(after) == "(")
		{
			const bool literal = after + 1 < tokens.size() && tokens[after + 1].kind == tkString &&
				(text_at(after + 2) == ")" || text_at(after + 2) == ",");
			if (!literal) add(lvReview, line, parts[0] + " of a computed string");
		}
		if (parts[0] == "_G" && text_at(after) == "[")
			add(lvReview, line, "indexes _G by a computed name");
		for (const SPattern& pattern : review)
			for (u32 k = 0; k < parts.size(); ++k)
				if (parts[k] == pattern.a) { add(lvReview, line, xr_string("uses ") + pattern.what); break; }
		const bool console = (parts.size() >= 2 && parts[parts.size() - 2] == "get_console" && parts.back() == "execute") ||
			(parts[0] == "get_console" && text_at(after + 2) == ":" && text_at(after + 3) == "execute");
		if (console) add(lvReview, line, "runs a console command");
		if (parts.size() == 2 && parts[0] == "string" && parts[1] == "char" && text_at(after) == "(")
		{
			int depth = 0;
			u32 commas = 0;
			for (size_t j = after; j < tokens.size(); ++j)
			{
				if (tokens[j].text == "(") ++depth;
				else if (tokens[j].text == ")") { if (--depth == 0) break; }
				else if (tokens[j].text == "," && depth == 1) ++commas;
			}
			if (commas >= 16) add(lvReview, line, NqUtil::Format("builds a string of %u characters with string.char", commas + 1));
		}
	}
	for (size_t i = 0; i < tokens.size(); ++i)
		if (tokens[i].kind == tkString && tokens[i].text.size() > 2000 && EncodedBlob(tokens[i].text))
			add(lvReview, tokens[i].line, NqUtil::Format("holds a long encoded blob (%u characters)", (u32)tokens[i].text.size()));
}

//==============================================================================
// the thumbnail (MOD_CATALOG 6.1)
//==============================================================================
bool EditorCatalog::ValidThumbnail(const void* data, size_t size)
{
	const u8* b = (const u8*)data;
	if (size < 128 || size > kThumbnailLimit || 0 != memcmp(b, "DDS ", 4)) return false;
	const auto at = [b](size_t offset) { u32 v; memcpy(&v, b + offset, 4); return v; };
	const u32 header = at(4), height = at(12), width = at(16), mips = at(28), format_size = at(76), format_flags = at(80);
	if (header != 124 || width != 256 || height != 256 || format_size != 32 || !(format_flags & 0x4)) return false;
	const u32 block = 0 == memcmp(b + 84, "DXT1", 4) ? 8 : 0 == memcmp(b + 84, "DXT5", 4) ? 16 : 0;
	const u32 levels = mips ? mips : 1;
	if (!block || (levels != 1 && levels != 9)) return false;
	size_t expected = 128;
	for (u32 level = 0, side = 256; level != levels; ++level, side = side > 1 ? side / 2 : 1)
	{
		const u32 blocks = side >= 4 ? (side + 3) / 4 : 1;
		expected += size_t(blocks) * blocks * block;
	}
	return size == expected;
}

bool EditorCatalog::MakeThumbnail(LPCSTR image, LPCSTR dds, xr_string& err)
{
	int w = 0, h = 0, channels = 0;
	stbi_uc* pixels = image ? stbi_load(image, &w, &h, &channels, 4) : 0;
	if (!pixels || w < 16 || h < 16)
	{
		if (pixels) stbi_image_free(pixels);
		err = xr_string("can't read the picture ") + (image ? image : "") + " (PNG, JPG, TGA or BMP)";
		return false;
	}
	// the middle square, averaged down (or spread up) to 256 x 256
	const int side = w < h ? w : h;
	const int left = (w - side) / 2, top = (h - side) / 2;
	const u32 target = 256;
	xr_vector<u32> data(target * target);
	bool alpha = false;
	const float scale = float(side) / float(target);
	for (u32 y = 0; y < target; ++y)
		for (u32 x = 0; x < target; ++x)
		{
			const float fx0 = left + x * scale, fy0 = top + y * scale;
			const int x0 = int(fx0), y0 = int(fy0);
			const int x1 = std::max(x0 + 1, int(fx0 + scale)), y1 = std::max(y0 + 1, int(fy0 + scale));
			u32 sum[4] = { 0, 0, 0, 0 }, count = 0;
			for (int sy = y0; sy < y1 && sy < h; ++sy)
				for (int sx = x0; sx < x1 && sx < w; ++sx)
				{
					const stbi_uc* p = pixels + (size_t(sy) * w + sx) * 4;
					for (int c = 0; c < 4; ++c) sum[c] += p[c];
					++count;
				}
			if (!count) count = 1;
			const u32 r = sum[0] / count, g = sum[1] / count, b = sum[2] / count, a = sum[3] / count;
			alpha |= a < 250;
			data[y * target + x] = (a << 24) | (r << 16) | (g << 8) | b;
		}
	stbi_image_free(pixels);

	STextureParams tp;
	tp.fmt = alpha ? STextureParams::tfDXT5 : STextureParams::tfDXT1;
	tp.width = target;
	tp.height = target;
	tp.type = STextureParams::ttImage;
	tp.flags.set(STextureParams::flGenerateMipMaps, TRUE);
	tp.flags.set(STextureParams::flHasAlpha, alpha);
	tp.mip_filter = STextureParams::kMIPFilterBox;
	::DeleteFileA(dds);
	if (!ImageLib.MakeGameTexture(dds, data.data(), tp)) { err = "the DXT compressor failed"; return false; }
	xr_string written;
	if (!ReadFileBytes(dds, kThumbnailLimit, written) || !ValidThumbnail(written.data(), written.size()))
	{
		err = "the compressor wrote a file the game would not take as a thumbnail";
		return false;
	}
	return true;
}

//==============================================================================
// VirusTotal (MOD_CATALOG 11)
//==============================================================================
EditorCatalog::SVtPolicy::SVtPolicy() : block_trusted(1), warn_trusted(1), block_others(5), warn_others(2)
{
	// the catalog's template settings: used until the index is read
	static const char* engines[] = { "microsoft", "kaspersky", "eset-nod32", "bitdefender", "avast", "avg", "sophos",
		"symantec", "malwarebytes", "trendmicro", "mcafee", "f-secure", "drweb", "emsisoft", "gdata", "panda", "avira",
		"fortinet", "crowdstrike", "sentinelone", "paloalto" };
	for (u32 i = 0; i < sizeof(engines) / sizeof(engines[0]); ++i)
		trusted.push_back(engines[i]);
}

void EditorCatalog::ReadVtPolicy(const xr_string& index_text, SVtPolicy& out)
{
	out = SVtPolicy();
	xr_string value;
	if (IniValue(index_text, "virustotal", "trusted", value))
	{
		out.trusted.clear();
		xr_string engine;
		for (size_t i = 0; i <= value.size(); ++i)
		{
			if (i < value.size() && value[i] != ',') { engine += char(tolower((unsigned char)value[i])); continue; }
			while (!engine.empty() && engine.back() == ' ') engine.pop_back();
			while (!engine.empty() && engine[0] == ' ') engine.erase(0, 1);
			if (!engine.empty()) out.trusted.push_back(engine);
			engine.clear();
		}
	}
	const auto number = [&index_text, &value](LPCSTR key, u32& field)
		{ if (IniValue(index_text, "virustotal", key, value) && !value.empty()) field = (u32)strtoul(value.c_str(), 0, 10); };
	number("block_trusted", out.block_trusted);
	number("warn_trusted", out.warn_trusted);
	number("block_others", out.block_others);
	number("warn_others", out.warn_others);
}

EditorCatalog::SVtVerdict EditorCatalog::VtVerdict(const xr_vector<SVtEngine>& results, const SVtPolicy& policy)
{
	SVtVerdict v;
	v.engines = (u32)results.size();
	for (u32 i = 0; i < results.size(); ++i)
	{
		xr_string engine = results[i].engine;
		for (size_t c = 0; c < engine.size(); ++c) engine[c] = char(tolower((unsigned char)engine[c]));
		bool trusted = false;
		for (u32 t = 0; t < policy.trusted.size() && !trusted; ++t) trusted = policy.trusted[t] == engine;
		if (results[i].category == "malicious")
		{
			v.flagged.push_back(results[i].engine);
			++(trusted ? v.trusted_malicious : v.other_malicious);
		}
		else if (results[i].category == "suspicious" && trusted)
		{
			v.flagged.push_back(results[i].engine);
			++v.trusted_suspicious;
		}
	}
	// a lone no-name detection is noise; a trusted engine's word is not (11)
	if (v.trusted_malicious >= policy.block_trusted || v.other_malicious >= policy.block_others)	v.verdict = "block";
	else if (v.trusted_suspicious >= policy.warn_trusted || v.other_malicious >= policy.warn_others)	v.verdict = "warn";
	else																						v.verdict = "clean";
	return v;
}

bool EditorCatalog::ListedKey(const xr_string& index_text, const xr_string& id, xr_string& key)
{
	key.clear();
	xr_vector<std::pair<xr_string, xr_string> > rows;
	IniSection(index_text, "mods", rows);
	for (u32 i = 0; i < rows.size(); ++i)
	{
		if (rows[i].first != id) continue;
		// github, key, version, date
		const size_t first = rows[i].second.find(',');
		if (first == xr_string::npos) return true;
		const size_t second = rows[i].second.find(',', first + 1);
		key = rows[i].second.substr(first + 1, second == xr_string::npos ? xr_string::npos : second - first - 1);
		while (!key.empty() && key[0] == ' ') key.erase(0, 1);
		while (!key.empty() && key.back() == ' ') key.pop_back();
		return true;
	}
	return false;
}

//==============================================================================
// ini text of the catalog files
//==============================================================================
namespace
{
void TrimSpaces(xr_string& s)
{
	while (!s.empty() && (s.back() == ' ' || s.back() == '\t' || s.back() == '\r')) s.pop_back();
	size_t a = 0;
	while (a < s.size() && (s[a] == ' ' || s[a] == '\t')) ++a;
	s.erase(0, a);
}

// a ';' outside quotes starts a comment
void StripComment(xr_string& value)
{
	bool quoted = false;
	for (size_t i = 0; i < value.size(); ++i)
	{
		if (value[i] == '"') quoted = !quoted;
		else if (value[i] == ';' && !quoted) { value.resize(i); break; }
	}
	TrimSpaces(value);
}
} // namespace

void EditorCatalog::IniSection(const xr_string& text, LPCSTR section, xr_vector<std::pair<xr_string, xr_string> >& out)
{
	out.clear();
	bool inside = false;
	for (size_t at = 0; at < text.size();)
	{
		size_t end = text.find('\n', at);
		if (end == xr_string::npos) end = text.size();
		xr_string line = text.substr(at, end - at);
		at = end + 1;
		TrimSpaces(line);
		if (line.empty() || line[0] == ';') continue;
		if (line[0] == '[')
		{
			const size_t close = line.find(']');
			xr_string name = line.substr(1, close == xr_string::npos ? xr_string::npos : close - 1);
			TrimSpaces(name);
			inside = name == section;
			continue;
		}
		const size_t eq = line.find('=');
		if (!inside || eq == xr_string::npos) continue;
		xr_string key = line.substr(0, eq), value = line.substr(eq + 1);
		TrimSpaces(key);
		StripComment(value);
		out.push_back(std::make_pair(key, value));
	}
}

bool EditorCatalog::IniValue(const xr_string& text, LPCSTR section, LPCSTR key, xr_string& out)
{
	xr_vector<std::pair<xr_string, xr_string> > rows;
	IniSection(text, section, rows);
	for (u32 i = 0; i < rows.size(); ++i)
		if (rows[i].first == key) { out = rows[i].second; return true; }
	out.clear();
	return false;
}

xr_string EditorCatalog::Unquote(const xr_string& value)
{
	if (value.size() < 2 || value[0] != '"' || value.back() != '"') return value;
	xr_string out;
	for (size_t i = 1; i + 1 < value.size(); ++i)
	{
		if (value[i] != '\\' || i + 2 >= value.size()) { out += value[i]; continue; }
		const char e = value[++i];
		out += e == 'n' ? '\n' : e == 't' ? '\t' : e;
	}
	return out;
}

int EditorCatalog::CompareVersions(LPCSTR a, LPCSTR b)
{
	u32 va[4] = {}, vb[4] = {};
	sscanf_s(a ? a : "", "%u.%u.%u.%u", &va[0], &va[1], &va[2], &va[3]);
	sscanf_s(b ? b : "", "%u.%u.%u.%u", &vb[0], &vb[1], &vb[2], &vb[3]);
	for (u32 i = 0; i < 4; ++i)
		if (va[i] != vb[i]) return va[i] < vb[i] ? -1 : 1;
	return 0;
}
