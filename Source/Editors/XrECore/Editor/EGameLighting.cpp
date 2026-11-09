//---------------------------------------------------------------------------
#include "stdafx.h"
#pragma hdrstop

#include "EGameLighting.h"
#include "EditorProject.h"

#if defined(USE_DX11)
#include "../../../xrRender/DX10/StateManager/dx10ShaderResourceStateCache.h"
#endif

ECORE_API EGameLighting GGameLighting;

namespace
{
// the slots common.h declares s_glit / s_groof at
const u32 kDataSlot = 15;
const u32 kRoofSlot = 14;

// "22:00:00" -> seconds of the day; -1 for a section that is not a time
int SectionSeconds(LPCSTR name)
{
	int h = 0, m = 0, s = 0;
	if (3 != sscanf_s(name, "%d:%d:%d", &h, &m, &s))
		return -1;
	return h * 3600 + m * 60 + s;
}

// the first three numbers of a key (hemisphere_color carries a fourth)
Fvector ReadColor(CInifile& ini, const shared_str& sect, LPCSTR key)
{
	Fvector v;
	v.set(0.f, 0.f, 0.f);
	if (ini.line_exist(sect.c_str(), key))
		sscanf_s(ini.r_string(sect.c_str(), key), "%f,%f,%f", &v.x, &v.y, &v.z);
	return v;
}

Fvector ReadSunDir(CInifile& ini, const shared_str& sect)
{
	// the way CEnvDescriptor::load builds it: altitude goes in as the heading
	Fvector d;
	d.set(0.f, -1.f, 0.f);
	if (ini.line_exist(sect.c_str(), "sun_altitude") && ini.line_exist(sect.c_str(), "sun_longitude"))
		d.setHP(deg2rad(ini.r_float(sect.c_str(), "sun_altitude")), deg2rad(ini.r_float(sect.c_str(), "sun_longitude")));
	return d;
}

void WeathersDir(string_path& dir)
{
	FS.update_path(dir, "$game_config$", "environment\\weathers\\");
}
}

EGameLighting::EGameLighting()
{
	ambient.set(0.f, 0.f, 0.f);
	hemi.set(0.f, 0.f, 0.f);
	sun_color.set(0.f, 0.f, 0.f);
	sun_dir.set(0.f, -1.f, 0.f);
	fog_color.set(0.f, 0.f, 0.f);
	roof_origin.set(0.f, 0.f);
}

bool EGameLighting::Enable()
{
	enabled = true;
	return weather_ok || Reload();
}

void EGameLighting::Disable()
{
	enabled = false;
	Unbind();
}

bool EGameLighting::Reload()
{
	LoadGameSettings();
	return LoadWeather();
}

bool EGameLighting::SetWeather(LPCSTR name, float at_hour)
{
	weather = name ? name : "";
	hour = at_hour;
	return LoadWeather();
}

void EGameLighting::ListWeathers(xr_vector<xr_string>& out) const
{
	out.clear();
	string_path dir, mask;
	WeathersDir(dir);
	sprintf_s(mask, "%s*.ltx", dir);
	// straight from the disk: the linked game's configs are an extracted cache the FS does not index
	WIN32_FIND_DATAA fd;
	HANDLE h = ::FindFirstFileA(mask, &fd);
	if (INVALID_HANDLE_VALUE == h)
		return;
	do
	{
		if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)
			continue;
		xr_string name = fd.cFileName;
		name.resize(name.size() - 4);
		out.push_back(name);
	} while (::FindNextFileA(h, &fd));
	::FindClose(h);
	std::sort(out.begin(), out.end());
}

xr_string EGameLighting::DefaultWeather(float at_hour) const
{
	xr_vector<xr_string> all;
	ListWeathers(all);
	auto has = [&all](LPCSTR name) { return std::find(all.begin(), all.end(), xr_string(name)) != all.end(); };

	// Atmosfear (level_weathers.script) names a clear night af3_<night brightness>_clear_<moon phase>: a new game has
	// the default "slight" nights, and the phase counts the days since 2010 (one more from noon) in an 8-day cycle -
	// so the first night of a new game starts from alife's start_date
	if (pSettings && pSettings->line_exist("alife", "start_date"))
	{
		int D = 0, M = 0, Y = 0;
		if (3 == sscanf_s(pSettings->r_string("alife", "start_date"), "%d.%d.%d", &D, &M, &Y))
		{
			static const int kMonths[12] = { 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31 };
			int day = 365 * (Y - 2010) + D;
			for (int m = 1; m < M && m <= 12; ++m)
				day += kMonths[m - 1];
			if (at_hour >= 12.f)
				++day;
			char name[64];
			sprintf_s(name, "af3_slight_clear_%d", ((day % 8) + 8) % 8);
			if (has(name))
				return name;
		}
	}
	for (const xr_string& n : all)
		if (strstr(n.c_str(), "clear"))
			return n;
	return all.empty() ? xr_string() : all.front();
}

bool EGameLighting::LoadWeather()
{
	weather_ok = false;
	weather_error.clear();
	if (weather.empty())
		weather = DefaultWeather(hour);
	if (weather.empty())
	{
		weather_error = "the game has no weathers (configs\\environment\\weathers)";
		return false;
	}

	string_path dir, file;
	WeathersDir(dir);
	sprintf_s(file, "%s%s.ltx", dir, weather.c_str());
	if (INVALID_FILE_ATTRIBUTES == ::GetFileAttributesA(file))
	{
		weather_error = "no weather '" + weather + "' in the game's configs";
		return false;
	}

	CInifile* ini = xr_new<CInifile>(file, TRUE);
	struct SKey { int t; shared_str name; };
	xr_vector<SKey> keys;
	for (CInifile::Sect* s : ini->sections())
	{
		const int t = SectionSeconds(s->Name.c_str());
		if (t >= 0)
			keys.push_back({ t, s->Name });
	}
	if (keys.empty())
	{
		xr_delete(ini);
		weather_error = "weather '" + weather + "' has no hh:mm:ss sections";
		return false;
	}
	std::sort(keys.begin(), keys.end(), [](const SKey& a, const SKey& b) { return a.t < b.t; });

	// the last key at or before the hour and the one after it, around midnight as the game does
	const int day = 24 * 3600;
	const int want = ((iFloor(hour * 3600.f) % day) + day) % day;
	u32 a = u32(keys.size()) - 1;
	for (u32 i = 0; i < keys.size(); ++i)
		if (keys[i].t <= want)
			a = i;
	const u32 b = (a + 1) % keys.size();
	const int span = ((keys[b].t - keys[a].t) % day + day) % day;
	const float f = span ? float(((want - keys[a].t) % day + day) % day) / float(span) : 0.f;

	ambient.lerp(ReadColor(*ini, keys[a].name, "ambient_color"), ReadColor(*ini, keys[b].name, "ambient_color"), f);
	hemi.lerp(ReadColor(*ini, keys[a].name, "hemisphere_color"), ReadColor(*ini, keys[b].name, "hemisphere_color"), f);
	sun_color.lerp(ReadColor(*ini, keys[a].name, "sun_color"), ReadColor(*ini, keys[b].name, "sun_color"), f);
	fog_color.lerp(ReadColor(*ini, keys[a].name, "fog_color"), ReadColor(*ini, keys[b].name, "fog_color"), f);
	sun_dir.lerp(ReadSunDir(*ini, keys[a].name), ReadSunDir(*ini, keys[b].name), f);
	if (sun_dir.square_magnitude() < EPS_S)
		sun_dir.set(0.f, -1.f, 0.f);
	sun_dir.normalize();
	xr_delete(ini);

	weather_ok = true;
	return true;
}

void EGameLighting::LoadGameSettings()
{
	// Call of Pripyat's defaults, for a game whose user.ltx does not say
	lumscale_sun = lumscale_hemi = lumscale_amb = 1.f;
	tonemap = true;
	tonemap_middlegray = 1.f;
	tonemap_lowlum = 0.0001f;
	tonemap_amount = 0.7f;

	LPCSTR root = EditorProject::Active() ? EditorProject::GameRoot() : nullptr;
	if (!root || !root[0])
		return;
	string_path file;
	sprintf_s(file, "%s\\appdata\\user.ltx", root);
	FILE* f = nullptr;
	if (fopen_s(&f, file, "rb") || !f)
		return;
	// console commands, one "name value" a line
	char line[512];
	while (fgets(line, sizeof(line), f))
	{
		char key[128] = {}, value[128] = {};
		if (2 != sscanf_s(line, "%127s %127s", key, u32(sizeof(key)), value, u32(sizeof(value))))
			continue;
		const float v = float(atof(value));
		if (!xr_strcmp(key, "r2_sun_lumscale"))				lumscale_sun = v;
		else if (!xr_strcmp(key, "r2_sun_lumscale_hemi"))		lumscale_hemi = v;
		else if (!xr_strcmp(key, "r2_sun_lumscale_amb"))		lumscale_amb = v;
		else if (!xr_strcmp(key, "r2_tonemap"))				tonemap = !xr_strcmp(value, "on");
		else if (!xr_strcmp(key, "r2_tonemap_middlegray"))	tonemap_middlegray = v;
		else if (!xr_strcmp(key, "r2_tonemap_lowlum"))		tonemap_lowlum = v;
		else if (!xr_strcmp(key, "r2_tonemap_amount"))		tonemap_amount = v;
	}
	fclose(f);
}

float EGameLighting::Exposure() const
{
	if (exposure > 0.f)
		return exposure;
	if (!tonemap)
		return 1.f;
	// before the first measured frame: a night scene's (mean luminance about 0.05)
	return auto_exposure > 0.f ? auto_exposure : TonemapScale(0.05f);
}

float EGameLighting::TonemapScale(float lum) const
{
	const float amount = tonemap ? tonemap_amount : 0.f;
	const float x = 1.f + (tonemap_middlegray - 1.f) * amount;
	const float z = 1.f + (tonemap_lowlum - 1.f) * amount;
	return clampr(x / _max(lum * amount + z, EPS_S), 0.7f, 20.f);
}

u32 EGameLighting::ClearColor() const
{
	const float e = Exposure();
	return color_rgba_f(_min(fog_color.x * e, 1.f), _min(fog_color.y * e, 1.f), _min(fog_color.z * e, 1.f), 1.f);
}

bool EGameLighting::Upload()
{
#if defined(USE_DX11)
	const u32 width = HEADER + 3 * MAX_LIGHTS;
	if (!m_Data)
	{
		D3D11_TEXTURE2D_DESC d = {};
		d.Width				= width;
		d.Height			= 1;
		d.MipLevels			= 1;
		d.ArraySize			= 1;
		d.Format			= DXGI_FORMAT_R32G32B32A32_FLOAT;
		d.SampleDesc.Count	= 1;
		d.Usage				= D3D11_USAGE_DYNAMIC;
		d.BindFlags			= D3D11_BIND_SHADER_RESOURCE;
		d.CPUAccessFlags	= D3D11_CPU_ACCESS_WRITE;
		if (FAILED(HW.pDevice->CreateTexture2D(&d, nullptr, &m_Data)) || !m_Data)
			return false;
		if (FAILED(HW.pDevice->CreateShaderResourceView(m_Data, nullptr, &m_DataView)) || !m_DataView)
		{
			_RELEASE(m_Data);
			return false;
		}
	}

	D3D11_MAPPED_SUBRESOURCE m = {};
	if (FAILED(HW.pContext->Map(m_Data, 0, D3D11_MAP_WRITE_DISCARD, 0, &m)))
		return false;
	Fvector4* t = (Fvector4*)m.pData;
	ZeroMemory(t, width * sizeof(Fvector4));
	const u32 count = _min(u32(lights.size()), u32(MAX_LIGHTS));
	m_DrawnExposure = Exposure();
	t[0].set(1.f, float(count), m_DrawnExposure, indoor_hemi);
	t[1].set(ambient.x * lumscale_amb, ambient.y * lumscale_amb, ambient.z * lumscale_amb, 0.f);
	t[2].set(hemi.x * lumscale_hemi, hemi.y * lumscale_hemi, hemi.z * lumscale_hemi, 0.f);
	t[3].set(sun_dir.x, sun_dir.y, sun_dir.z, 0.f);
	t[4].set(sun_color.x * lumscale_sun, sun_color.y * lumscale_sun, sun_color.z * lumscale_sun, 0.f);
	if (roof_cells)
		t[5].set(roof_origin.x, roof_origin.y, 1.f / ROOF_CELL, float(roof_cells));
	for (u32 i = 0; i < count; ++i)
	{
		const SGameLightSource& L = lights[i];
		Fvector4* p = t + HEADER + i * 3;
		p[0].set(L.position.x, L.position.y, L.position.z, _max(L.range, 0.01f));
		p[1].set(L.color.x, L.color.y, L.color.z, L.spot_cos);
		p[2].set(L.direction.x, L.direction.y, L.direction.z, 0.f);
	}
	HW.pContext->Unmap(m_Data, 0);

	if (roof_changed)
	{
		roof_changed = false;
		_RELEASE(m_RoofView);
		_RELEASE(m_Roof);
		if (roof_cells && roof.size() == roof_cells * roof_cells)
		{
			D3D11_TEXTURE2D_DESC d = {};
			d.Width				= roof_cells;
			d.Height			= roof_cells;
			d.MipLevels			= 1;
			d.ArraySize			= 1;
			d.Format			= DXGI_FORMAT_R32_FLOAT;
			d.SampleDesc.Count	= 1;
			d.Usage				= D3D11_USAGE_IMMUTABLE;
			d.BindFlags			= D3D11_BIND_SHADER_RESOURCE;
			D3D11_SUBRESOURCE_DATA init = {};
			init.pSysMem		= roof.data();
			init.SysMemPitch	= roof_cells * sizeof(float);
			if (SUCCEEDED(HW.pDevice->CreateTexture2D(&d, &init, &m_Roof)) && m_Roof)
				HW.pDevice->CreateShaderResourceView(m_Roof, nullptr, &m_RoofView);
		}
	}
	return true;
#else
	return false;
#endif
}

void EGameLighting::Bind()
{
#if defined(USE_DX11)
	if (!enabled || !weather_ok || !Upload())
		return;
	// through the state cache, so a range it applies later puts the same views back; and straight away, so nothing
	// drawn before that apply runs unlit
	SRVSManager.SetPSResource(kDataSlot, m_DataView);
	SRVSManager.SetPSResource(kRoofSlot, m_RoofView);
	ID3D11ShaderResourceView* views[2] = { m_RoofView, m_DataView };
	HW.pContext->PSSetShaderResources(kRoofSlot, 2, views);
	m_Bound = true;
#endif
}

void EGameLighting::Unbind()
{
#if defined(USE_DX11)
	if (!m_Bound)
		return;
	SRVSManager.SetPSResource(kDataSlot, nullptr);
	SRVSManager.SetPSResource(kRoofSlot, nullptr);
	ID3D11ShaderResourceView* none[2] = { nullptr, nullptr };
	HW.pContext->PSSetShaderResources(kRoofSlot, 2, none);
	m_Bound = false;
#endif
}

bool EGameLighting::Measure(ID3D11Texture2D* target, u32 w, u32 h)
{
#if defined(USE_DX11)
	if (!m_Bound || exposure > 0.f || !tonemap || !target || !w || !h || m_DrawnExposure <= 0.f)
		return false;
	D3D11_TEXTURE2D_DESC d = {};
	target->GetDesc(&d);
	const bool bgra = d.Format == DXGI_FORMAT_B8G8R8A8_UNORM || d.Format == DXGI_FORMAT_B8G8R8X8_UNORM;
	const bool rgba = d.Format == DXGI_FORMAT_R8G8B8A8_UNORM;
	if ((!bgra && !rgba) || d.SampleDesc.Count != 1)
		return false;
	w = _min(w, d.Width);
	h = _min(h, d.Height);

	D3D11_TEXTURE2D_DESC sd = {};
	if (m_Staging)
		m_Staging->GetDesc(&sd);
	if (!m_Staging || sd.Width != d.Width || sd.Height != d.Height || sd.Format != d.Format)
	{
		_RELEASE(m_Staging);
		sd = d;
		sd.MipLevels		= 1;
		sd.ArraySize		= 1;
		sd.Usage			= D3D11_USAGE_STAGING;
		sd.BindFlags		= 0;
		sd.CPUAccessFlags	= D3D11_CPU_ACCESS_READ;
		sd.MiscFlags		= 0;
		if (FAILED(HW.pDevice->CreateTexture2D(&sd, nullptr, &m_Staging)) || !m_Staging)
			return false;
	}
	HW.pContext->CopyResource(m_Staging, target);
	D3D11_MAPPED_SUBRESOURCE m = {};
	if (FAILED(HW.pContext->Map(m_Staging, 0, D3D11_MAP_READ, 0, &m)))
		return false;

	// the game's measure: the plain mean of dot(rgb, (0.3, 0.38, 0.22)) over the frame before the tonemap, i.e.
	// what was drawn divided by the exposure it was drawn with; a grid of about 200 x 120 samples is plenty
	const u32 step_x = _max(w / 200u, 1u), step_y = _max(h / 120u, 1u);
	const u32 ri = bgra ? 2 : 0, bi = bgra ? 0 : 2;
	double sum = 0.0;
	u32 count = 0;
	for (u32 y = step_y / 2; y < h; y += step_y)
	{
		const u8* row = (const u8*)m.pData + size_t(m.RowPitch) * y;
		for (u32 x = step_x / 2; x < w; x += step_x)
		{
			const u8* px = row + x * 4;
			sum += 0.3 * px[ri] + 0.38 * px[1] + 0.22 * px[bi];
			++count;
		}
	}
	HW.pContext->Unmap(m_Staging, 0);
	if (!count)
		return false;

	scene_lum = float(sum / (255.0 * count)) / m_DrawnExposure;
	const float next = TonemapScale(scene_lum);
	const bool moved = auto_exposure <= 0.f || _abs(next - auto_exposure) > 0.03f * auto_exposure;
	auto_exposure = next;
	return moved;
#else
	return false;
#endif
}

void EGameLighting::Destroy()
{
	Unbind();
#if defined(USE_DX11)
	_RELEASE(m_Staging);
	_RELEASE(m_DataView);
	_RELEASE(m_Data);
	_RELEASE(m_RoofView);
	_RELEASE(m_Roof);
#endif
	roof_changed = !roof.empty();
}
