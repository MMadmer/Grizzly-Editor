#include "stdafx.h"
#include "GameLightingScene.h"
#include "..\..\XrECore\Editor\EGameLighting.h"
#include "..\..\XrECore\Editor\Nq\NqUtil.h"
#include "..\..\..\xrServerEntities\xrServer_Objects_ALife_All.h"

// SceneObject.cpp: lets a raypick through objects the camera does not see
extern bool g_RayPickAll;

namespace
{
SGameLightSource MakeLight(LPCSTR kind, CCustomObject* owner, const Fvector& position, float range, const Fvector& color)
{
	SGameLightSource L;
	L.position	= position;
	L.range		= range;
	L.color		= color;
	L.spot_cos	= 0.f;
	L.direction.set(0.f, -1.f, 0.f);
	L.kind		= kind;
	L.owner		= owner && owner->GetName() ? owner->GetName() : "";
	return L;
}

// a light animation's colour over its length, the way the game reads it (CalculateBGR, the channels swapped back)
bool AnimColor(LPCSTR name, Fvector& out)
{
	CLAItem* item = name && name[0] ? LALib.FindItem(name) : nullptr;
	if (!item || item->iFrameCount <= 0)
		return false;
	const int samples = 16;
	Fvector sum;
	sum.set(0.f, 0.f, 0.f);
	for (int i = 0; i < samples; ++i)
	{
		int frame = 0;
		const u32 c = item->CalculateBGR(item->Length_sec() * float(i) / float(samples), frame);
		sum.x += float(color_get_B(c));
		sum.y += float(color_get_G(c));
		sum.z += float(color_get_R(c));
	}
	out.mul(sum, 1.f / (255.f * float(samples)));
	return true;
}

// CHangingLamp: the main light on the light bone (a spot along the bone's +Z, or a point) and, when the lamp asks for
// it, a point "ambient" light on its ambient bone. Only lamps the R2+ renderers draw count.
void AddLamp(xr_vector<SGameLightSource>& out, CSpawnPoint* sp, CSE_ALifeObjectHangingLamp* lamp)
{
	if (!lamp->flags.is(CSE_ALifeObjectHangingLamp::flR2))
		return;
	Fvector color;
	if (AnimColor(lamp->color_animator.c_str(), color))
		color.mul(lamp->brightness);
	else
	{
		Fcolor c;
		c.set(lamp->color);
		color.set(c.r * lamp->brightness, c.g * lamp->brightness, c.b * lamp->brightness);
	}

	Fmatrix xf;
	sp->m_SpawnData.get_bone_xform(lamp->light_main_bone.c_str(), xf);
	xf.mulA_43(sp->_Transform());
	SGameLightSource L = MakeLight("lamp", sp, xf.c, lamp->range, color);
	if (lamp->flags.is(CSE_ALifeObjectHangingLamp::flTypeSpot))
	{
		L.spot_cos = _cos(lamp->spot_cone_angle * 0.5f);
		L.direction.set(xf.k);
		L.direction.normalize_safe();
	}
	out.push_back(L);

	if (lamp->flags.is(CSE_ALifeObjectHangingLamp::flPointAmbient) && lamp->m_ambient_radius > 0.f)
	{
		Fmatrix axf;
		sp->m_SpawnData.get_bone_xform(lamp->light_ambient_bone.c_str(), axf);
		axf.mulA_43(sp->_Transform());
		Fvector ambient;
		ambient.mul(color, lamp->m_ambient_power);
		out.push_back(MakeLight("lamp_ambient", sp, axf.c, lamp->m_ambient_radius, ambient));
	}
}

// a campfire burns at night: the zone's idle light (CCustomZone) over it, coloured by its light animation
void AddCampfire(xr_vector<SGameLightSource>& out, CSpawnPoint* sp, LPCSTR section)
{
	if (!section || !strstr(section, "campfire") || !pSettings->section_exist(section))
		return;
	if (!pSettings->line_exist(section, "idle_light") || !pSettings->r_bool(section, "idle_light"))
		return;
	const float range = pSettings->line_exist(section, "idle_light_range") ? pSettings->r_float(section, "idle_light_range") : 0.f;
	if (range <= 0.f)
		return;
	Fvector color;
	if (!pSettings->line_exist(section, "idle_light_anim") || !AnimColor(pSettings->r_string(section, "idle_light_anim"), color))
		color.set(1.f, 0.6f, 0.3f);
	Fvector position = sp->GetPosition();
	if (pSettings->line_exist(section, "idle_light_height"))
		position.y += pSettings->r_float(section, "idle_light_height");
	out.push_back(MakeLight("campfire", sp, position, range, color));
}

// every light the game would draw in the level, wherever the camera is
void Collect(xr_vector<SGameLightSource>& all)
{
	all.clear();
	// the level's own lights the R2+ renderers draw at run time: its dynamic point lights (CLight_DB::Load)
	for (CCustomObject* obj : Scene->ListObj(OBJCLASS_LIGHT))
	{
		CLight* l = (CLight*)obj;
		if (!l->Visible() || l->m_Type != ELight::ltPoint || !l->m_Flags.is(ELight::flAffectDynamic))
			continue;
		Fvector color;
		color.set(l->m_Color.r * l->m_Brightness, l->m_Color.g * l->m_Brightness, l->m_Color.b * l->m_Brightness);
		all.push_back(MakeLight("level", l, l->GetPosition(), l->m_Range, color));
	}
	for (CCustomObject* obj : Scene->ListObj(OBJCLASS_SPAWNPOINT))
	{
		CSpawnPoint* sp = (CSpawnPoint*)obj;
		ISE_Abstract* e = sp->Visible() ? sp->m_SpawnData.m_Data : nullptr;
		if (!e)
			continue;
		if (CSE_ALifeObjectHangingLamp* lamp = dynamic_cast<CSE_ALifeObjectHangingLamp*>(e))
			AddLamp(all, sp, lamp);
		else
			AddCampfire(all, sp, e->name());
	}
}

void Gather(EGameLighting& G)
{
	xr_vector<SGameLightSource> all;
	Collect(all);
	// the ones whose reach the camera sees, nearest first
	const Fvector& eye = EDevice->vCameraPosition;
	G.lights.clear();
	for (const SGameLightSource& L : all)
		if (L.range > 0.f && ::Render->ViewBase.testSphere_dirty(const_cast<Fvector&>(L.position), L.range))
			G.lights.push_back(L);
	G.lights_in_view = u32(G.lights.size());
	std::sort(G.lights.begin(), G.lights.end(), [&eye](const SGameLightSource& a, const SGameLightSource& b)
		{ return eye.distance_to_sqr(a.position) < eye.distance_to_sqr(b.position); });
	if (G.lights.size() > EGameLighting::MAX_LIGHTS)
		G.lights.resize(EGameLighting::MAX_LIGHTS);
}

// The roof map around the camera: for every cell the height of the topmost static surface over it, straight down
// from above the region. The camera may wander a quarter of the map before it moves.
void BakeRoof(EGameLighting& G, const Fvector& eye)
{
	const u32 cells = EGameLighting::ROOF_CELLS;
	const float cell = EGameLighting::ROOF_CELL;
	const float half = float(cells) * cell * 0.5f;
	if (G.roof_cells == cells && _abs(eye.x - (G.roof_origin.x + half)) < half * 0.5f &&
		_abs(eye.z - (G.roof_origin.y + half)) < half * 0.5f)
		return;

	CTimer timer;
	timer.Start();
	Fvector2 origin;
	origin.set(floorf(eye.x / 32.f) * 32.f - half, floorf(eye.z / 32.f) * 32.f - half);
	const float x1 = origin.x + float(cells) * cell, z1 = origin.y + float(cells) * cell;

	// static geometry over the region, bucketed by 16 x 16 tiles of it
	struct SCandidate { CSceneObject* object; Fbox box; };
	xr_vector<SCandidate> candidates;
	float top = -1e4f;
	for (CCustomObject* obj : Scene->ListObj(OBJCLASS_SCENEOBJECT))
	{
		CSceneObject* so = (CSceneObject*)obj;
		Fbox box;
		// a bare visual would be picked by its box, a whole level's terrain included
		if (!so->GetReference() || so->IsDynamic() || !so->GetBox(box))
			continue;
		if (box.max.x < origin.x || box.min.x > x1 || box.max.z < origin.y || box.min.z > z1)
			continue;
		candidates.push_back({ so, box });
		top = _max(top, box.max.y);
	}
	const u32 tiles = 16, per = cells / tiles;
	const float tile = float(per) * cell;
	xr_vector<xr_vector<u32>> buckets(tiles * tiles);
	for (u32 i = 0; i < candidates.size(); ++i)
	{
		const Fbox& b = candidates[i].box;
		const int tx0 = clampr(iFloor((b.min.x - origin.x) / tile), 0, int(tiles) - 1);
		const int tx1 = clampr(iFloor((b.max.x - origin.x) / tile), 0, int(tiles) - 1);
		const int tz0 = clampr(iFloor((b.min.z - origin.y) / tile), 0, int(tiles) - 1);
		const int tz1 = clampr(iFloor((b.max.z - origin.y) / tile), 0, int(tiles) - 1);
		for (int tz = tz0; tz <= tz1; ++tz)
			for (int tx = tx0; tx <= tx1; ++tx)
				buckets[tz * tiles + tx].push_back(i);
	}

	G.roof.assign(cells * cells, -1e4f);
	const bool pick_all = g_RayPickAll;
	g_RayPickAll = true;
	Fvector down;
	down.set(0.f, -1.f, 0.f);
	const float start = top + 1.f, reach = start + 1e4f;
	for (u32 z = 0; z < cells; ++z)
		for (u32 x = 0; x < cells; ++x)
		{
			Fvector S;
			S.set(origin.x + (float(x) + 0.5f) * cell, start, origin.y + (float(z) + 0.5f) * cell);
			float dist = reach;
			for (u32 i : buckets[(z / per) * tiles + x / per])
			{
				const Fbox& b = candidates[i].box;
				// outside the box's column, or entirely below what was hit already
				if (S.x < b.min.x || S.x > b.max.x || S.z < b.min.z || S.z > b.max.z || start - b.max.y >= dist)
					continue;
				candidates[i].object->RayPick(dist, S, down, nullptr);
			}
			if (dist < reach)
				G.roof[z * cells + x] = start - dist;
		}
	g_RayPickAll = pick_all;

	G.roof_origin = origin;
	G.roof_cells = cells;
	G.roof_changed = true;
	G.roof_bake_ms = timer.GetElapsed_ms();
	Msg("* game lighting: roof map %ux%u around (%.0f, %.0f) from %u objects in %u ms", cells, cells,
		origin.x + half, origin.y + half, u32(candidates.size()), G.roof_bake_ms);
}

void Prepare(EGameLighting& G)
{
	if (!G.enabled || !Scene || !Scene->valid())
		return;
	if (!G.weather_ok)
		return;
	BakeRoof(G, EDevice->vCameraPosition);
	Gather(G);
}

void JsonVec(xr_string& out, LPCSTR key, const Fvector& v)
{
	char tmp[160];
	sprintf_s(tmp, ",\"%s\":[%.4f,%.4f,%.4f]", key, v.x, v.y, v.z);
	out += tmp;
}

LPCSTR HourText(float hour)
{
	static string16 text;
	const int minutes = iFloor(hour * 60.f + 0.5f);
	sprintf_s(text, "%02d:%02d", (minutes / 60) % 24, minutes % 60);
	return text;
}
}

void GameLighting_BeforeRender()
{
	EGameLighting& G = GGameLighting;
	if (!G.enabled)
		return;
	Prepare(G);
	G.Bind();
}

void GameLighting_AfterRender()
{
	EGameLighting& G = GGameLighting;
	// the viewport's own pass adapts the exposure, the way the game's eye does (a preview capture draws elsewhere)
	if (G.enabled && UI && UI->RT && UI->RT->pRT && RCache.get_RT() == UI->RT->pRT &&
		G.Measure(UI->RT->pSurface, UI->GetRenderWidth(), UI->GetRenderHeight()))
		UI->RedrawScene();
	G.Unbind();
}

void GameLighting_Rebake()
{
	GGameLighting.roof_cells = 0;
	GGameLighting.roof.clear();
	GGameLighting.roof_changed = true;
}

void GameLighting_JsonState(xr_string& out)
{
	EGameLighting& G = GGameLighting;
	char tmp[512];
	out += G.enabled ? "\"enabled\":true,\"weather\":" : "\"enabled\":false,\"weather\":";
	NqUtil::JsonString(out, G.weather);
	sprintf_s(tmp, ",\"hour\":%.2f,\"clock\":\"%s\",\"weather_ok\":%s,\"error\":", G.hour, HourText(G.hour),
		G.weather_ok ? "true" : "false");
	out += tmp;
	NqUtil::JsonString(out, G.weather_error);
	JsonVec(out, "ambient", G.ambient);
	JsonVec(out, "hemi", G.hemi);
	JsonVec(out, "sun_color", G.sun_color);
	JsonVec(out, "sun_dir", G.sun_dir);
	JsonVec(out, "fog_color", G.fog_color);
	sprintf_s(tmp,
		",\"lumscale\":{\"sun\":%.3f,\"hemi\":%.3f,\"amb\":%.3f},"
		"\"tonemap\":{\"on\":%s,\"middlegray\":%.3f,\"lowlum\":%.4f,\"amount\":%.3f},"
		"\"exposure\":%.3f,\"exposure_setting\":%.3f,\"auto_exposure\":%.3f,\"scene_lum\":%.4f,"
		"\"indoor_hemi\":%.3f",
		G.lumscale_sun, G.lumscale_hemi, G.lumscale_amb, G.tonemap ? "true" : "false", G.tonemap_middlegray,
		G.tonemap_lowlum, G.tonemap_amount, G.Exposure(), G.exposure, G.auto_exposure, G.scene_lum, G.indoor_hemi);
	out += tmp;

	u32 level = 0, lamp = 0, ambient = 0, campfire = 0;
	for (const SGameLightSource& L : G.lights)
	{
		if (!xr_strcmp(L.kind, "level"))			++level;
		else if (!xr_strcmp(L.kind, "lamp"))		++lamp;
		else if (!xr_strcmp(L.kind, "lamp_ambient"))	++ambient;
		else										++campfire;
	}
	sprintf_s(tmp,
		",\"lights\":{\"drawn\":%u,\"in_view\":%u,\"level\":%u,\"lamp\":%u,\"lamp_ambient\":%u,\"campfire\":%u},"
		"\"roof\":{\"cells\":%u,\"cell\":%.2f,\"origin\":[%.1f,%.1f],\"bake_ms\":%u}",
		u32(G.lights.size()), G.lights_in_view, level, lamp, ambient, campfire, G.roof_cells,
		EGameLighting::ROOF_CELL, G.roof_origin.x, G.roof_origin.y, G.roof_bake_ms);
	out += tmp;
}

void GameLighting_Mcp(LPCSTR raw, xr_string& out)
{
	EGameLighting& G = GGameLighting;
	bool weather_changed = false;
	xr_string text;
	float value = 0.f;
	if (NqUtil::ArgString(raw, "weather", text))
	{
		// empty = the default for the hour (the first night of a new game)
		G.weather = text;
		weather_changed = true;
	}
	if (NqUtil::ArgFloat(raw, "hour", value))
	{
		G.hour = value;
		weather_changed = true;
	}
	if (NqUtil::ArgFloat(raw, "exposure", value))
		G.exposure = _max(value, 0.f);
	if (NqUtil::ArgFloat(raw, "indoor_hemi", value))
		G.indoor_hemi = clampr(value, 0.f, 1.f);
	if (NqUtil::ArgBool(raw, "reload", false))
		G.Reload();
	else if (weather_changed)
		G.SetWeather(G.weather.c_str(), G.hour);
	if (NqUtil::ArgBool(raw, "rebake", false))
		GameLighting_Rebake();
	const bool want = NqUtil::ArgBool(raw, "enabled", G.enabled);
	if (want != G.enabled)
	{
		if (want)
			G.Enable();
		else
			G.Disable();
	}
	// the answer names what the next frame draws
	Prepare(G);
	UI->RedrawScene();

	out = G.enabled && !G.weather_ok ? "{\"ok\":false," : "{\"ok\":true,";
	GameLighting_JsonState(out);
	if (NqUtil::ArgBool(raw, "weathers", false))
	{
		xr_vector<xr_string> names;
		G.ListWeathers(names);
		out += ",\"weathers\":";
		NqUtil::JsonStringArray(out, names);
	}
	const bool everywhere = NqUtil::ArgBool(raw, "all", false);
	if (everywhere || NqUtil::ArgBool(raw, "list", false))
	{
		// the lights drawn now, or with all=true every light of the level (planning lamps from above)
		xr_vector<SGameLightSource> level;
		if (everywhere)
			Collect(level);
		const xr_vector<SGameLightSource>& list = everywhere ? level : G.lights;
		out += ",\"light_list\":[";
		char tmp[384];
		for (u32 i = 0; i < list.size(); ++i)
		{
			const SGameLightSource& L = list[i];
			out += i ? ",{\"kind\":\"" : "{\"kind\":\"";
			out += L.kind;
			out += "\",\"owner\":";
			NqUtil::JsonString(out, L.owner.size() ? L.owner.c_str() : "");
			sprintf_s(tmp, ",\"pos\":[%.2f,%.2f,%.2f],\"range\":%.2f,\"color\":[%.3f,%.3f,%.3f],\"spot_cos\":%.3f}",
				L.position.x, L.position.y, L.position.z, L.range, L.color.x, L.color.y, L.color.z, L.spot_cos);
			out += tmp;
		}
		out += "]";
	}
	out += "}";
}

void GameLighting_Menu()
{
	EGameLighting& G = GGameLighting;
	{
		bool on = G.enabled;
		if (XUI::MenuItem("Enabled", nullptr, &on))
		{
			if (on)
				G.Enable();
			else
				G.Disable();
			UI->RedrawScene();
		}
	}
	if (G.enabled && !G.weather_ok)
		XUI::MenuItem(G.weather_error.c_str(), nullptr, false, false);
	ImGui::Separator();

	static xr_vector<xr_string> s_Weathers;
	if (ImGui::BeginMenu("Weather"))
	{
		// read as the submenu opens: the linked game may have changed
		if (ImGui::IsWindowAppearing())
			G.ListWeathers(s_Weathers);
		if (XUI::MenuItem("Default (a new game's first night)", nullptr, false))
		{
			G.SetWeather("", G.hour);
			UI->RedrawScene();
		}
		ImGui::Separator();
		for (const xr_string& name : s_Weathers)
			if (XUI::MenuItem(name.c_str(), nullptr, name == G.weather))
			{
				G.SetWeather(name.c_str(), G.hour);
				UI->RedrawScene();
			}
		ImGui::EndMenu();
	}
	if (ImGui::BeginMenu("Hour"))
	{
		for (int i = 0; i < 24; ++i)
		{
			// the evening first: the night is what this view is for
			const float h = float((i + 18) % 24);
			if (XUI::MenuItem(HourText(h), nullptr, fsimilar(G.hour, h)))
			{
				G.SetWeather(G.weather.c_str(), h);
				UI->RedrawScene();
			}
		}
		ImGui::EndMenu();
	}
	if (ImGui::BeginMenu("Exposure"))
	{
		if (XUI::MenuItem("Game Tonemap", nullptr, G.exposure <= 0.f))
		{
			G.exposure = 0.f;
			UI->RedrawScene();
		}
		static const float kExposures[] = { 1.f, 1.5f, 2.f, 3.f, 4.f };
		for (float e : kExposures)
		{
			char label[32];
			sprintf_s(label, "x%.1f", e);
			if (XUI::MenuItem(label, nullptr, fsimilar(G.exposure, e)))
			{
				G.exposure = e;
				UI->RedrawScene();
			}
		}
		ImGui::EndMenu();
	}
	if (ImGui::BeginMenu("Indoor Sky Light"))
	{
		static const float kShares[] = { 0.f, 0.05f, 0.1f, 0.15f, 0.25f, 0.5f, 1.f };
		for (float s : kShares)
		{
			char label[32];
			sprintf_s(label, "%d%%", iFloor(s * 100.f + 0.5f));
			if (XUI::MenuItem(label, nullptr, fsimilar(G.indoor_hemi, s)))
			{
				G.indoor_hemi = s;
				UI->RedrawScene();
			}
		}
		ImGui::EndMenu();
	}
	ImGui::Separator();
	if (XUI::MenuItem("Rebake Roof Map", nullptr, false, G.enabled))
	{
		GameLighting_Rebake();
		UI->RedrawScene();
	}
	if (G.enabled && G.weather_ok)
	{
		char info[160];
		sprintf_s(info, "%u lights, %s at %s", u32(G.lights.size()), G.weather.c_str(), HourText(G.hour));
		XUI::MenuItem(info, nullptr, false, false);
	}
}
