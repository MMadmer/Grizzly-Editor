#include "stdafx.h"
#include "EditorAssetIcons.h"
#include "ui_main.h"
#include "Nq/NqUtil.h"
#include "../../XrEUI/XrUITheme.h"

namespace
{
//------------------------------------------------------------------------------
// the shapes
//
// Everything lives in a unit square. A stroke is a polyline - arcs and circles
// are tessellated when the icon is built, so a draw is a scale and an offset and
// nothing else.
//------------------------------------------------------------------------------
struct SStroke
{
	xr_vector<ImVec2>	pts;
	bool				closed;
	bool				dashed;		// drawn broken: a template, not a thing
	bool				secondary;	// the quieter colour
	float				weight;		// multiplies the base thickness

						SStroke() : closed(false), dashed(false), secondary(false), weight(1.f) {}
};

struct SRect
{
	ImVec2	a, b;
	float	rounding;
	bool	filled;

			SRect() : rounding(0.f), filled(false) {}
};

struct SDot
{
	ImVec2	c;
	float	r;

			SDot() : r(0.f) {}
};

struct SIcon
{
	xr_vector<SStroke>			strokes;
	xr_vector<SRect>			rects;
	xr_vector<SDot>				dots;
	xr_vector<xr_vector<ImVec2> > fills;	// filled polygons (an arrow head)
	bool						built;

								SIcon() : built(false) {}
};

SIcon	s_Icons[EditorAssetIcons::kCharacterTemplate + 1];

// One scratch buffer for every polyline this unit hands to ImGui. The uses do
// not overlap, and a fresh vector per stroke would be an allocation per tile
// per frame - which is the one thing a cached icon must not cost.
ImVector<ImVec2>	s_Screen;

ImVec2 P(float x, float y) { return ImVec2(x, y); }

void Line(SIcon& icon, const ImVec2* pts, int n, bool secondary = false, float weight = 1.f,
		  bool dashed = false, bool closed = false)
{
	SStroke s;
	s.pts.assign(pts, pts + n);
	s.secondary	= secondary;
	s.weight	= weight;
	s.dashed	= dashed;
	s.closed	= closed;
	icon.strokes.push_back(s);
}

// An arc, tessellated once. y grows downward, so an angle of 1.5*PI is the top.
void Arc(SIcon& icon, float cx, float cy, float r, float a0, float a1, int steps,
		 bool secondary = false, float weight = 1.f, bool dashed = false)
{
	SStroke s;
	s.pts.reserve(steps + 1);
	for (int i = 0; i <= steps; ++i)
	{
		const float a = a0 + (a1 - a0) * (float(i) / float(steps));
		s.pts.push_back(ImVec2(cx + r * _cos(a), cy + r * _sin(a)));
	}
	s.secondary	= secondary;
	s.weight	= weight;
	s.dashed	= dashed;
	icon.strokes.push_back(s);
}

//------------------------------------------------------------------------------
// A quest is a flow: one node splitting into two. That is what the graph editor
// shows, so that is what the file should look like.
//------------------------------------------------------------------------------
void BuildQuest(SIcon& icon)
{
	SRect a;	a.a = P(0.28f, 0.03f);	a.b = P(0.72f, 0.23f);	a.rounding = 0.055f;
	SRect b;	b.a = P(0.02f, 0.63f);	b.b = P(0.44f, 0.83f);	b.rounding = 0.055f;
	SRect c;	c.a = P(0.56f, 0.63f);	c.b = P(0.98f, 0.83f);	c.rounding = 0.055f;
	icon.rects.push_back(a);
	icon.rects.push_back(b);
	icon.rects.push_back(c);

	const ImVec2 trunk[] = { P(0.50f, 0.23f), P(0.50f, 0.44f) };
	Line(icon, trunk, 2, true);

	// the split, drawn as one elbow so the corners meet cleanly
	const ImVec2 split[] = { P(0.23f, 0.63f), P(0.23f, 0.44f), P(0.77f, 0.44f), P(0.77f, 0.63f) };
	Line(icon, split, 4, true);
}

//------------------------------------------------------------------------------
// A behaviour graph moves people: a route with its waypoints and an arrow at the end.
//------------------------------------------------------------------------------
void BuildBehaviour(SIcon& icon)
{
	const ImVec2 route[] = { P(0.07f, 0.82f), P(0.30f, 0.44f), P(0.55f, 0.63f), P(0.753f, 0.286f) };
	Line(icon, route, 4, false, 0.72f);

	// fatter than the line they sit on, or they read as kinks and not as places
	SDot d0;	d0.c = P(0.07f, 0.82f);	d0.r = 0.093f;	icon.dots.push_back(d0);
	SDot d1;	d1.c = P(0.30f, 0.44f);	d1.r = 0.093f;	icon.dots.push_back(d1);
	SDot d2;	d2.c = P(0.55f, 0.63f);	d2.r = 0.093f;	icon.dots.push_back(d2);

	// the head, pointing along the last leg
	xr_vector<ImVec2> head;
	head.push_back(P(0.86f, 0.12f));
	head.push_back(P(0.826f, 0.330f));
	head.push_back(P(0.672f, 0.242f));
	icon.fills.push_back(head);
}

//------------------------------------------------------------------------------
// A character is a person: the head and the shoulders, nothing else. `dashed`
// is the template - the same person, drawn as an outline, because it never
// becomes anybody in the game.
//------------------------------------------------------------------------------
void BuildCharacter(SIcon& icon, bool dashed)
{
	Arc(icon, 0.50f, 0.30f, 0.185f, 0.f, PI_MUL_2, 28, false, 1.f, dashed);
	Arc(icon, 0.50f, 0.98f, 0.36f, PI, PI_MUL_2, 26, false, 1.f, dashed);
}

const SIcon& Icon(EditorAssetIcons::EKind kind)
{
	SIcon& icon = s_Icons[kind];
	if (icon.built) return icon;
	icon.built = true;
	switch (kind)
	{
	case EditorAssetIcons::kQuest:				BuildQuest(icon);				break;
	case EditorAssetIcons::kBehaviour:			BuildBehaviour(icon);			break;
	case EditorAssetIcons::kCharacter:			BuildCharacter(icon, false);	break;
	case EditorAssetIcons::kCharacterTemplate:	BuildCharacter(icon, true);		break;
	default:																	break;
	}
	return icon;
}

//------------------------------------------------------------------------------
// what a file is
//
// The extension answers it for a quest (.nqasset) and for a behaviour graph
// (.behasset) - that is the point of giving each its own. Only an `.npcasset`
// has to be looked INTO, to tell a template from a character; that is in the
// first line or two of a canonical file, so a kilobyte is plenty - and the
// answer is kept, because a tile that stats its file every frame is exactly the
// kind of thing the startup profile was written to stop.
//------------------------------------------------------------------------------
u32											s_Generation = 1;
xr_flat_hash_map<xr_string, u32>			s_CachedGen;
xr_flat_hash_map<xr_string, u8>				s_CachedKind;

// `key = "value"` or `key = true`, as the canonical writer spells them and as a
// hand-written file is likely to
bool HeadSays(const char* text, u32 len, LPCSTR key, LPCSTR value)
{
	const u32 klen = (u32)xr_strlen(key);
	for (u32 i = 0; i + klen < len; ++i)
	{
		if (i && (isalnum((u8)text[i - 1]) || text[i - 1] == '_'))	continue;
		if (0 != _strnicmp(text + i, key, klen))					continue;
		u32 j = i + klen;
		if (j < len && (isalnum((u8)text[j]) || text[j] == '_'))		continue;
		while (j < len && (text[j] == ' ' || text[j] == '\t'))		++j;
		if (j >= len || text[j] != '=')								continue;
		++j;
		while (j < len && (text[j] == ' ' || text[j] == '\t' || text[j] == '"'))	++j;
		const u32 vlen = (u32)xr_strlen(value);
		if (j + vlen <= len && 0 == _strnicmp(text + j, value, vlen))	return true;
	}
	return false;
}

// The theme answers in 0xRRGGBB with no alpha; ImGui wants it packed the other
// way round WITH one, and an alpha of zero is why nothing showed up at first.
u32 Col(XFinedTheme::ColorToken token, float alpha)
{
	const u32 rgb = XFinedTheme::Rgb(token);
	return IM_COL32((rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff, int(alpha * 255.f + 0.5f));
}

bool ReadHead(LPCSTR path, char* buf, u32 cap, u32& got)
{
	got = 0;
	HANDLE h = ::CreateFileA(path, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, NULL,
		OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL | FILE_FLAG_SEQUENTIAL_SCAN, NULL);
	if (INVALID_HANDLE_VALUE == h) return false;
	DWORD read = 0;
	const BOOL ok = ::ReadFile(h, buf, cap - 1, &read, NULL);
	::CloseHandle(h);
	if (!ok) return false;
	buf[read] = 0;
	got = (u32)read;
	return true;
}
} // namespace

//------------------------------------------------------------------------------
LPCSTR EditorAssetIcons::OwnExtension(LPCSTR name)
{
	LPCSTR ext = name ? strrchr(name, '.') : 0;
	// a dot in a folder of the path is not an extension, and an extension alone is not a name
	if (!ext || ext == name || strpbrk(ext, "\\/") || '\\' == ext[-1] || '/' == ext[-1])	return 0;
	if (0 == _stricmp(ext, ".nqasset") || 0 == _stricmp(ext, ".behasset") || 0 == _stricmp(ext, ".npcasset"))
		return ext;
	return 0;
}

EditorAssetIcons::EKind EditorAssetIcons::KindOf(LPCSTR path)
{
	if (!path || !path[0]) return kNone;

	LPCSTR ext = strrchr(path, '.');
	if (!ext) return kNone;
	// no file is touched for these two: the name is the whole answer
	if (0 == _stricmp(ext, ".nqasset"))		return kQuest;
	if (0 == _stricmp(ext, ".behasset"))	return kBehaviour;
	if (0 != _stricmp(ext, ".npcasset"))	return kNone;

	xr_string key = path;
	xr_strlwr(&key[0]);

	// cached, unless the generation moved on
	auto gen = s_CachedGen.find(key);
	if (gen != s_CachedGen.end() && gen->second == s_Generation)
	{
		auto k = s_CachedKind.find(key);
		if (k != s_CachedKind.end()) return (EKind)k->second;
	}

	EKind kind = kCharacter;
	char head[1024];
	u32	 got = 0;
	if (ReadHead(path, head, sizeof(head), got) && got && HeadSays(head, got, "abstract", "true"))
		kind = kCharacterTemplate;

	s_CachedGen[key]	= s_Generation;
	s_CachedKind[key]	= (u8)kind;
	return kind;
}

void EditorAssetIcons::Invalidate()
{
	++s_Generation;
	// the maps are kept: the entries are overwritten as the tiles come back, and
	// a browser refresh must not turn into a burst of allocations
	if (s_CachedGen.size() > 4096)
	{
		s_CachedGen.clear();
		s_CachedKind.clear();
	}
}

LPCSTR EditorAssetIcons::Name(EKind kind)
{
	switch (kind)
	{
	case kQuest:				return "quest";
	case kBehaviour:			return "behaviour graph";
	case kCharacter:			return "character";
	case kCharacterTemplate:	return "character template";
	default:					return "";
	}
}

void EditorAssetIcons::Draw(EKind kind, float x0, float y0, float x1, float y1)
{
	if (kind == kNone) return;
	const SIcon& icon = Icon(kind);

	// the unit square sits inside the rectangle with room to breathe, and stays
	// square when the rectangle is not
	const float	w		= x1 - x0;
	const float	h		= y1 - y0;
	const float	side	= (w < h ? w : h) * 0.58f;
	if (side < 4.f) return;
	const float	ox		= x0 + (w - side) * 0.5f;
	const float	oy		= y0 + (h - side) * 0.5f;

	ImDrawList*	dl		= ImGui::GetWindowDrawList();
	const u32	primary	= Col(XFinedTheme::ColorToken::Text, 0.92f);
	const u32	quiet	= Col(XFinedTheme::ColorToken::Muted, 0.85f);
	const float	thick	= _max(1.35f, side * 0.075f);

	for (u32 i = 0; i < icon.rects.size(); ++i)
	{
		const SRect& r = icon.rects[i];
		const ImVec2 a(ox + r.a.x * side, oy + r.a.y * side);
		const ImVec2 b(ox + r.b.x * side, oy + r.b.y * side);
		if (r.filled)	dl->AddRectFilled(a, b, primary, r.rounding * side);
		else			dl->AddRect(a, b, primary, r.rounding * side, 0, thick);
	}

	for (u32 i = 0; i < icon.strokes.size(); ++i)
	{
		const SStroke&	s	= icon.strokes[i];
		const u32		col	= s.secondary ? quiet : primary;
		const float		t	= thick * s.weight;
		if (s.dashed)
		{
			// Dashes are measured ALONG the stroke, and each one is flushed as a
			// SINGLE polyline. Both halves matter: an arc arrives as ~28 short
			// pieces, so "every other piece" is noise that gets finer the bigger
			// the icon is drawn, and a dash emitted piece by piece frays at every
			// join instead of being mitred.
			const float	dash	= side * 0.17f;
			const float	period	= dash + side * 0.095f;
			float		phase	= 0.f;
			ImVector<ImVec2>& run = s_Screen;
			run.resize(0);
			for (u32 p = 0; p + 1 < s.pts.size(); ++p)
			{
				const ImVec2 a(ox + s.pts[p].x * side, oy + s.pts[p].y * side);
				const ImVec2 b(ox + s.pts[p + 1].x * side, oy + s.pts[p + 1].y * side);
				const float	 len = _sqrt((b.x - a.x) * (b.x - a.x) + (b.y - a.y) * (b.y - a.y));
				if (len < EPS_S) continue;
				for (float done = 0.f; done < len; )
				{
					const bool	on		= phase < dash;
					const float take	= _min((on ? dash : period) - phase, len - done);
					if (on)
					{
						const float u0 = done / len, u1 = (done + take) / len;
						if (run.empty())
							run.push_back(ImVec2(a.x + (b.x - a.x) * u0, a.y + (b.y - a.y) * u0));
						run.push_back(ImVec2(a.x + (b.x - a.x) * u1, a.y + (b.y - a.y) * u1));
					}
					done	+= take;
					phase	+= take;
					// the dash ends the moment the walk leaves it
					if (!run.empty() && phase >= dash - EPS_S)
					{
						dl->AddPolyline(run.Data, run.Size, col, ImDrawFlags_None, t);
						run.resize(0);
					}
					if (phase >= period - EPS_S) phase = 0.f;
				}
			}
			if (!run.empty())
				dl->AddPolyline(run.Data, run.Size, col, ImDrawFlags_None, t);
			continue;
		}
		// one polyline: the joins are mitred by ImGui, so an elbow has no gap
		ImVector<ImVec2>& screen = s_Screen;
		screen.resize((int)s.pts.size());
		for (u32 p = 0; p < s.pts.size(); ++p)
			screen[(int)p] = ImVec2(ox + s.pts[p].x * side, oy + s.pts[p].y * side);
		dl->AddPolyline(screen.Data, screen.Size, col,
			s.closed ? ImDrawFlags_Closed : ImDrawFlags_None, t);
	}

	for (u32 i = 0; i < icon.dots.size(); ++i)
	{
		const SDot& d = icon.dots[i];
		dl->AddCircleFilled(ImVec2(ox + d.c.x * side, oy + d.c.y * side), d.r * side, primary, 0);
	}

	for (u32 i = 0; i < icon.fills.size(); ++i)
	{
		const xr_vector<ImVec2>& poly = icon.fills[i];
		ImVector<ImVec2>& screen = s_Screen;
		screen.resize((int)poly.size());
		for (u32 p = 0; p < poly.size(); ++p)
			screen[(int)p] = ImVec2(ox + poly[p].x * side, oy + poly[p].y * side);
		dl->AddConvexPolyFilled(screen.Data, screen.Size, primary);
	}
}

void EditorAssetIcons::DrawFor(LPCSTR path, float x0, float y0, float x1, float y1)
{
	Draw(KindOf(path), x0, y0, x1, y1);
}
