#include "stdafx.h"
#include "XrUIIcons.h"
#include "XrUIKit.h"
#include <algorithm>
#include <cmath>
#include <initializer_list>

namespace
{
using XUI::Icon;

//------------------------------------------------------------------------------
// the geometry
//
// Every glyph lives on a 16-unit grid with a 1-unit margin and is stroked 1.5 units
// wide. The whole set is built ONCE, on the first draw: arcs are tessellated then,
// fills get their winding and convexity then, corners that want a round join are
// found then. A draw is a scale, an offset and - at 20 px and below - a snap to the
// pixel grid, and nothing else.
//------------------------------------------------------------------------------
constexpr float		kGrid		= 16.f;
constexpr float		kStroke		= 1.5f;		// units
constexpr float		kSnapUpTo	= 20.f;		// px: at this size and below, points land on pixels
constexpr float		kQuiet		= 0.55f;	// a secondary stroke's alpha, relative to the colour
constexpr float		kRoundAbove	= 2.f;		// px of stroke above which joins and caps are drawn round
constexpr float		kJoinCos	= 0.94f;	// a turn sharper than ~20 degrees is a corner
constexpr double	kDeg		= 3.14159265358979323846 / 180.0;

// per point
enum : u8
{
	pfFree	= 1,	// a point of a curve: never snapped, or the curve wobbles
	pfJoin	= 2,	// a corner or an end: covered by a disc when joins are drawn round
};

// per primitive
enum : u8
{
	kClosed	= 1,	// a dashed path is closed by repeating its first point, and keeps the flag
	kQuietP	= 2,
	kDashed	= 4,
	kFilled	= 8,
	kConvex	= 16,
};

enum class EPrim : u8 { Stroke, Rect, Circle, Fill };

struct SPrim
{
	EPrim	type	= EPrim::Stroke;
	u8		flags	= 0;
	float	r		= 0.f;		// rect rounding, circle radius
	ImVec2	a, b;				// rect corners; a circle's centre is a
	u32		first	= 0;		// a stroke's or a fill's points in SGlyph::pts
	u32		count	= 0;
};

struct SGlyph
{
	xr_vector<SPrim>	prims;
	xr_vector<ImVec2>	pts;
	xr_vector<u8>		pflags;
};

SGlyph	s_Glyphs[(int)Icon::Count];
bool	s_Built = false;

// Scratch buffers for every polyline handed to ImGui. Icons are drawn in every
// toolbar and row each frame; a vector per stroke would be an allocation each time.
ImVector<ImVec2>	s_Screen;
ImVector<ImVec2>	s_Run;

// Coordinates are written to two decimals, so an arc that should start where a line ends can miss
// it by a hundredth: that is one point, or the path steps back over a sliver and a mitre spikes.
bool Near(const ImVec2& a, const ImVec2& b)
{
	return fabsf(a.x - b.x) < 0.02f && fabsf(a.y - b.y) < 0.02f;
}

//------------------------------------------------------------------------------
// the builder: what the glyphs below are written in. Coordinates are grid units,
// angles are degrees clockwise from +x (y grows downward, so 270 is the top).
//------------------------------------------------------------------------------
class B
{
public:
	explicit B(SGlyph& g) : m_G(g) {}

	// both apply to what is added after them
	B&		Quiet(bool on = true)	{ m_Quiet = on; return *this; }
	B&		Dashed(bool on = true)	{ m_Dashed = on; return *this; }

	// a polyline through x,y pairs; Z closes it
	void	L(std::initializer_list<double> xy)	{ Poly(xy.begin(), xy.size(), false); }
	void	Z(std::initializer_list<double> xy)	{ Poly(xy.begin(), xy.size(), true); }
	// a filled polygon; FZ strokes its outline too, so its corners round like every other
	void	F(std::initializer_list<double> xy)	{ Fill(xy.begin(), xy.size()); }
	void	FZ(std::initializer_list<double> xy)	{ F(xy); Z(xy); }
	void	R(double x0, double y0, double x1, double y1, double r = 1.0)	{ Rect(x0, y0, x1, y1, r, false); }
	void	RF(double x0, double y0, double x1, double y1, double r = 1.0)	{ Rect(x0, y0, x1, y1, r, true); }
	void	C(double cx, double cy, double r)	{ Circle(cx, cy, r, false); }
	void	D(double cx, double cy, double r)	{ Circle(cx, cy, r, true); }
	// an arc; a1 below a0 runs counter-clockwise
	void	A(double cx, double cy, double r, double a0, double a1)	{ M().Arc(cx, cy, r, a0, a1).End(); }
	void	E(double cx, double cy, double rx, double ry)				{ M().Ellipse(cx, cy, rx, ry, 0.0, 360.0).End(true); }
	// an arrow head: a chevron with its tip at x,y, pointing along `deg`
	void	Head(double x, double y, double deg, double len);
	void	Arrow(double x0, double y0, double x1, double y1, double head = 3.5);
	// an elliptic arc with an arrow head at its a1 end
	void	ArcArrow(double cx, double cy, double rx, double ry, double a0, double a1, double head);

	// paths: M starts one, To / Arc / Ellipse extend it, End finishes it
	B&		M()						{ m_Path.clear(); m_Flags.clear(); return *this; }
	B&		M(double x, double y)	{ return M().To(x, y); }
	B&		To(double x, double y)	{ Push(x, y, 0); return *this; }
	B&		Arc(double cx, double cy, double r, double a0, double a1)	{ return Ellipse(cx, cy, r, r, a0, a1); }
	B&		Ellipse(double cx, double cy, double rx, double ry, double a0, double a1);
	void	End(bool closed = false);

	void	Poly(const double* xy, size_t n, bool closed);
	void	Fill(const double* xy, size_t n);

private:
	void	Push(double x, double y, u8 flags);
	void	Rect(double x0, double y0, double x1, double y1, double r, bool filled);
	void	Circle(double cx, double cy, double r, bool filled);
	u8		Style() const	{ return u8((m_Quiet ? kQuietP : 0) | (m_Dashed ? kDashed : 0)); }

	SGlyph&				m_G;
	bool				m_Quiet		= false;
	bool				m_Dashed	= false;
	xr_vector<ImVec2>	m_Path;
	xr_vector<u8>		m_Flags;
};

void B::Push(double x, double y, u8 flags)
{
	const ImVec2 p((float)x, (float)y);
	// a path that meets itself (a line into an arc) keeps one point, snapped unless both are curve
	if (!m_Path.empty() && Near(m_Path.back(), p))
	{
		m_Flags.back() &= flags;
		return;
	}
	m_Path.push_back(p);
	m_Flags.push_back(flags);
}

B& B::Ellipse(double cx, double cy, double rx, double ry, double a0, double a1)
{
	const int steps = std::max(2, (int)ceil(fabs(a1 - a0) / 10.0));
	for (int i = 0; i <= steps; ++i)
	{
		const double a = (a0 + (a1 - a0) * i / steps) * kDeg;
		Push(cx + rx * cos(a), cy + ry * sin(a), pfFree);
	}
	return *this;
}

void B::End(bool closed)
{
	if (closed && m_Path.size() > 2 && Near(m_Path.back(), m_Path.front()))
	{
		m_Flags.front() &= m_Flags.back();
		m_Path.pop_back();
		m_Flags.pop_back();
	}
	if (m_Path.size() < 2)
	{
		M();
		return;
	}
	// a dash walker needs a start and an end: a closed dashed path runs back to its start
	const bool loop = m_Dashed && closed;
	if (loop)
	{
		m_Path.push_back(m_Path.front());
		m_Flags.push_back(m_Flags.front());
		closed = false;
	}

	const u32 n = (u32)m_Path.size();
	for (u32 i = 0; i < n; ++i)
	{
		if (closed || (i && i + 1 < n))
		{
			const ImVec2& a = m_Path[(i + n - 1) % n];
			const ImVec2& p = m_Path[i];
			const ImVec2& c = m_Path[(i + 1) % n];
			const float ux = p.x - a.x, uy = p.y - a.y, vx = c.x - p.x, vy = c.y - p.y;
			const float lu = sqrtf(ux * ux + uy * uy), lv = sqrtf(vx * vx + vy * vy);
			if (lu < 1e-4f || lv < 1e-4f || (ux * vx + uy * vy) / (lu * lv) > kJoinCos)
				continue;
		}
		m_Flags[i] |= pfJoin;
	}

	SPrim prim;
	prim.type	= EPrim::Stroke;
	prim.flags	= u8(Style() | (closed || loop ? kClosed : 0));
	prim.first	= (u32)m_G.pts.size();
	prim.count	= n;
	m_G.pts.insert(m_G.pts.end(), m_Path.begin(), m_Path.end());
	m_G.pflags.insert(m_G.pflags.end(), m_Flags.begin(), m_Flags.end());
	m_G.prims.push_back(prim);
	M();
}

void B::Poly(const double* xy, size_t n, bool closed)
{
	M();
	for (size_t i = 0; i + 1 < n; i += 2)
		To(xy[i], xy[i + 1]);
	End(closed);
}

void B::Fill(const double* xy, size_t n)
{
	const u32 first = (u32)m_G.pts.size();
	const u32 count = (u32)(n / 2);
	if (count < 3)
		return;
	for (size_t i = 0; i + 1 < n; i += 2)
	{
		m_G.pts.push_back(ImVec2((float)xy[i], (float)xy[i + 1]));
		m_G.pflags.push_back(0);
	}
	ImVec2* p = &m_G.pts[first];

	// ImGui anti-aliases a fill only when it runs clockwise on screen: a positive area with y down
	float area = 0.f;
	for (u32 i = 0; i < count; ++i)
	{
		const ImVec2& a = p[i];
		const ImVec2& b = p[(i + 1) % count];
		area += a.x * b.y - b.x * a.y;
	}
	if (area < 0.f)
		std::reverse(p, p + count);

	bool convex = true;
	for (u32 i = 0; i < count && convex; ++i)
	{
		const ImVec2& a = p[i];
		const ImVec2& b = p[(i + 1) % count];
		const ImVec2& c = p[(i + 2) % count];
		convex = (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x) >= -1e-4f;
	}

	SPrim prim;
	prim.type	= EPrim::Fill;
	prim.flags	= u8((m_Quiet ? kQuietP : 0) | kFilled | (convex ? kConvex : 0));
	prim.first	= first;
	prim.count	= count;
	m_G.prims.push_back(prim);
}

void B::Rect(double x0, double y0, double x1, double y1, double r, bool filled)
{
	// PathRect cannot dash: a dashed one becomes a path
	if (m_Dashed && !filled)
	{
		M().Arc(x0 + r, y0 + r, r, 180.0, 270.0).Arc(x1 - r, y0 + r, r, 270.0, 360.0)
			.Arc(x1 - r, y1 - r, r, 0.0, 90.0).Arc(x0 + r, y1 - r, r, 90.0, 180.0).End(true);
		return;
	}
	SPrim prim;
	prim.type	= EPrim::Rect;
	prim.flags	= u8(Style() | (filled ? kFilled : 0));
	prim.r		= (float)r;
	prim.a		= ImVec2((float)x0, (float)y0);
	prim.b		= ImVec2((float)x1, (float)y1);
	m_G.prims.push_back(prim);
}

void B::Circle(double cx, double cy, double r, bool filled)
{
	if (m_Dashed && !filled)
	{
		M().Arc(cx, cy, r, 0.0, 360.0).End(true);
		return;
	}
	SPrim prim;
	prim.type	= EPrim::Circle;
	prim.flags	= u8(Style() | (filled ? kFilled : 0));
	prim.r		= (float)r;
	prim.a		= ImVec2((float)cx, (float)cy);
	m_G.prims.push_back(prim);
}

void B::Head(double x, double y, double deg, double len)
{
	const double back0 = (deg + 180.0 - 42.0) * kDeg, back1 = (deg + 180.0 + 42.0) * kDeg;
	M(x + len * cos(back0), y + len * sin(back0)).To(x, y).To(x + len * cos(back1), y + len * sin(back1)).End();
}

void B::Arrow(double x0, double y0, double x1, double y1, double head)
{
	L({ x0, y0, x1, y1 });
	Head(x1, y1, atan2(y1 - y0, x1 - x0) / kDeg, head);
}

void B::ArcArrow(double cx, double cy, double rx, double ry, double a0, double a1, double head)
{
	M().Ellipse(cx, cy, rx, ry, a0, a1).End();
	// the tangent at the end, in the direction of travel
	const double a = a1 * kDeg, s = a1 >= a0 ? 1.0 : -1.0;
	Head(cx + rx * cos(a), cy + ry * sin(a), atan2(s * ry * cos(a), -s * rx * sin(a)) / kDeg, head);
}

//------------------------------------------------------------------------------
// shapes more than one glyph is made of
//------------------------------------------------------------------------------
void Page(B& b)
{
	b.Z({ 3.5, 1.5, 9.5, 1.5, 12.5, 4.5, 12.5, 14.5, 3.5, 14.5 });
	b.L({ 9.5, 1.5, 9.5, 4.5, 12.5, 4.5 });
}

// a page whose lower right corner is left to a badge
void PageCut(B& b)
{
	b.L({ 8.0, 14.5, 3.5, 14.5, 3.5, 1.5, 9.5, 1.5, 12.5, 4.5, 12.5, 7.0 });
	b.L({ 9.5, 1.5, 9.5, 4.5, 12.5, 4.5 });
}

void Folder(B& b)
{
	b.Z({ 1.5, 3.0, 6.0, 3.0, 7.5, 4.75, 14.5, 4.75, 14.5, 13.5, 1.5, 13.5 });
}

void Clipboard(B& b)
{
	b.R(5.5, 1.5, 10.5, 4.25, 0.75);
	b.M(10.5, 3.0).To(11.5, 3.0).Arc(11.5, 4.0, 1.0, 270.0, 360.0).To(12.5, 13.5).Arc(11.5, 13.5, 1.0, 0.0, 90.0)
		.To(4.5, 14.5).Arc(4.5, 13.5, 1.0, 90.0, 180.0).To(3.5, 4.0).Arc(4.5, 4.0, 1.0, 180.0, 270.0).To(5.5, 3.0).End();
}

void TwoSheets(B& b)
{
	b.R(5.5, 5.5, 14.0, 14.0, 1.5);
	b.M(3.5, 10.5).To(3.0, 10.5).Arc(3.0, 9.5, 1.0, 90.0, 180.0).To(2.0, 3.0).Arc(3.0, 3.0, 1.0, 180.0, 270.0)
		.To(9.5, 2.0).Arc(9.5, 3.0, 1.0, 270.0, 360.0).To(10.5, 3.5).End();
}

void Corners(B& b)
{
	b.L({ 2.0, 5.5, 2.0, 2.0, 5.5, 2.0 });
	b.L({ 10.5, 2.0, 14.0, 2.0, 14.0, 5.5 });
	b.L({ 14.0, 10.5, 14.0, 14.0, 10.5, 14.0 });
	b.L({ 5.5, 14.0, 2.0, 14.0, 2.0, 10.5 });
}

void Lens(B& b, double cx, double cy, double r, double reach)
{
	const double d = r * 0.7071;
	b.C(cx, cy, r);
	b.L({ cx + d, cy + d, reach, reach });
}

void EyeShape(B& b)
{
	// two arcs of a circle 7.17 wide in radius, meeting at the corners of the eye
	b.M().Arc(8.0, 11.03, 7.17, 205.0, 335.0).Arc(8.0, 4.97, 7.17, 25.0, 155.0).End(true);
}

// head and shoulders, the shoulders a rounded box
void Bust(B& b, double cx, double head_y, double head_r, double top, double half, double bottom)
{
	const double r = std::min(half, 2.5);
	b.C(cx, head_y, head_r);
	b.M(cx - half, bottom).To(cx - half, top + r).Arc(cx - half + r, top + r, r, 180.0, 270.0).To(cx + half - r, top)
		.Arc(cx + half - r, top + r, r, 270.0, 360.0).To(cx + half, bottom).End();
}

void Regular(B& b, double cx, double cy, double r, int n, double deg0)
{
	double xy[32];
	for (int i = 0; i < n; ++i)
	{
		const double a = (deg0 + i * 360.0 / n) * kDeg;
		xy[i * 2]		= cx + r * cos(a);
		xy[i * 2 + 1]	= cy + r * sin(a);
	}
	b.Poly(xy, n * 2, true);
}

void Star(B& b, double cx, double cy, double ro, double ri, int n, bool filled)
{
	double xy[32];
	for (int i = 0; i < n * 2; ++i)
	{
		const double a = (-90.0 + i * 180.0 / n) * kDeg, r = i & 1 ? ri : ro;
		xy[i * 2]		= cx + r * cos(a);
		xy[i * 2 + 1]	= cy + r * sin(a);
	}
	if (filled)
		b.Fill(xy, n * 4);
	b.Poly(xy, n * 4, true);
}

void Gear(B& b, double cx, double cy, double ro, double ri, int n)
{
	double xy[64];
	const double p = 180.0 / n;
	for (int i = 0; i < n; ++i)
	{
		const double base = -90.0 + i * 2.0 * p;
		// flank, the tooth's top, flank; the valley to the next tooth is a chord
		const double a[] = { base - 0.8 * p, base - 0.42 * p, base + 0.42 * p, base + 0.8 * p };
		const double r[] = { ri, ro, ro, ri };
		for (int k = 0; k < 4; ++k)
		{
			xy[(i * 4 + k) * 2]		= cx + r[k] * cos(a[k] * kDeg);
			xy[(i * 4 + k) * 2 + 1]	= cy + r[k] * sin(a[k] * kDeg);
		}
	}
	b.Poly(xy, n * 8, true);
}

// the angle of x,y seen from cx,cy, in [0, 360)
double AngleOf(double cx, double cy, double x, double y)
{
	const double a = atan2(y - cy, x - cx) / kDeg;
	return a < 0.0 ? a + 360.0 : a;
}

// where two circles meet, the upper point of the two
void Meet(double x1, double y1, double r1, double x2, double y2, double r2, double& x, double& y)
{
	const double dx = x2 - x1, dy = y2 - y1, d = sqrt(dx * dx + dy * dy);
	const double a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d), h = sqrt(std::max(0.0, r1 * r1 - a * a));
	const double mx = x1 + a * dx / d, my = y1 + a * dy / d;
	const double px = -dy / d * h, py = dx / d * h;
	if (my - py < my + py)	{ x = mx - px; y = my - py; }
	else					{ x = mx + px; y = my + py; }
}

// a cloud: three bumps on a flat base
void Cloud(B& b)
{
	const double lx = 5.0, ly = 8.0, lr = 2.5, tx = 8.75, ty = 6.25, tr = 3.5, rx = 12.0, ry = 8.5, rr = 2.0;
	double ax, ay, bx, by;
	Meet(lx, ly, lr, tx, ty, tr, ax, ay);
	Meet(tx, ty, tr, rx, ry, rr, bx, by);
	double t0 = AngleOf(tx, ty, ax, ay), t1 = AngleOf(tx, ty, bx, by);
	if (t1 < t0)
		t1 += 360.0;
	b.M().Arc(lx, ly, lr, 90.0, AngleOf(lx, ly, ax, ay)).Arc(tx, ty, tr, t0, t1)
		.Arc(rx, ry, rr, AngleOf(rx, ry, bx, by), 450.0).End(true);
}

// the editor's own asset kinds keep the shapes they were drawn with in a unit square
double U(double u)
{
	return 1.0 + 14.0 * u;
}

//------------------------------------------------------------------------------
// the glyphs
//------------------------------------------------------------------------------
void Build(Icon id, B& b)
{
	switch (id)
	{
	// chrome ------------------------------------------------------------------
	case Icon::Minimize:		b.L({ 3.5, 8.0, 12.5, 8.0 }); break;
	case Icon::Maximize:		b.R(3.5, 3.5, 12.5, 12.5, 1.0); break;
	case Icon::Restore:
		b.R(3.0, 5.5, 10.5, 13.0, 1.0);
		b.L({ 5.5, 5.5, 5.5, 3.0, 13.0, 3.0, 13.0, 10.5, 10.5, 10.5 });
		break;
	case Icon::Close:
		b.L({ 4.0, 4.0, 12.0, 12.0 });
		b.L({ 12.0, 4.0, 4.0, 12.0 });
		break;
	case Icon::ChevronUp:		b.L({ 4.0, 10.0, 8.0, 6.0, 12.0, 10.0 }); break;
	case Icon::ChevronDown:		b.L({ 4.0, 6.0, 8.0, 10.0, 12.0, 6.0 }); break;
	case Icon::ChevronLeft:		b.L({ 10.0, 4.0, 6.0, 8.0, 10.0, 12.0 }); break;
	case Icon::ChevronRight:	b.L({ 6.0, 4.0, 10.0, 8.0, 6.0, 12.0 }); break;
	// the one filled triangle of the chrome: that is how a menu caret reads
	case Icon::Caret:			b.F({ 4.5, 6.25, 11.5, 6.25, 8.0, 10.25 }); break;
	case Icon::More:
		b.D(3.5, 8.0, 1.25);
		b.D(8.0, 8.0, 1.25);
		b.D(12.5, 8.0, 1.25);
		break;
	case Icon::Overflow:
		b.L({ 3.5, 4.5, 7.0, 8.0, 3.5, 11.5 });
		b.L({ 8.5, 4.5, 12.0, 8.0, 8.5, 11.5 });
		break;
	case Icon::Menu:
		b.L({ 2.5, 4.0, 13.5, 4.0 });
		b.L({ 2.5, 8.0, 13.5, 8.0 });
		b.L({ 2.5, 12.0, 13.5, 12.0 });
		break;
	case Icon::Dot:				b.D(8.0, 8.0, 2.25); break;
	case Icon::Grip:
		for (double y : { 3.5, 8.0, 12.5 })
		{
			b.D(6.0, y, 1.15);
			b.D(10.0, y, 1.15);
		}
		break;
	case Icon::Pin:
		b.L({ 5.0, 2.0, 11.0, 2.0 });
		b.L({ 6.0, 2.0, 6.0, 6.5, 3.5, 9.5, 12.5, 9.5, 10.0, 6.5, 10.0, 2.0 });
		b.L({ 8.0, 9.5, 8.0, 14.5 });
		break;
	case Icon::External:
		b.L({ 7.0, 3.5, 3.5, 3.5, 3.5, 12.5, 12.5, 12.5, 12.5, 9.0 });
		b.L({ 7.5, 8.5, 13.5, 2.5 });
		b.L({ 9.5, 2.5, 13.5, 2.5, 13.5, 6.5 });
		break;
	case Icon::SidebarLeft:
		b.R(1.5, 2.5, 14.5, 13.5, 1.5);
		b.L({ 6.0, 2.5, 6.0, 13.5 });
		break;
	case Icon::SidebarRight:
		b.R(1.5, 2.5, 14.5, 13.5, 1.5);
		b.L({ 10.0, 2.5, 10.0, 13.5 });
		break;
	case Icon::PanelBottom:
		b.R(1.5, 2.5, 14.5, 13.5, 1.5);
		b.L({ 1.5, 9.5, 14.5, 9.5 });
		break;

	// actions -----------------------------------------------------------------
	case Icon::New:
		Page(b);
		b.L({ 8.0, 7.5, 8.0, 12.5 });
		b.L({ 5.5, 10.0, 10.5, 10.0 });
		break;
	case Icon::Open:
		Folder(b);
		b.Arrow(8.0, 11.75, 8.0, 7.0, 2.75);
		break;
	case Icon::Save:
		b.Z({ 2.5, 2.5, 11.0, 2.5, 13.5, 5.0, 13.5, 13.5, 2.5, 13.5 });
		b.L({ 5.0, 2.5, 5.0, 5.5, 10.0, 5.5, 10.0, 2.5 });
		b.L({ 4.5, 13.5, 4.5, 9.0, 11.5, 9.0, 11.5, 13.5 });
		break;
	case Icon::SaveAll:
		b.Z({ 1.5, 4.5, 9.0, 4.5, 11.5, 7.0, 11.5, 14.5, 1.5, 14.5 });
		b.L({ 4.0, 4.5, 4.0, 7.25, 8.0, 7.25, 8.0, 4.5 });
		b.L({ 3.5, 14.5, 3.5, 10.5, 9.5, 10.5, 9.5, 14.5 });
		b.L({ 4.5, 4.5, 4.5, 1.5, 12.0, 1.5, 14.5, 4.0, 14.5, 11.5, 11.5, 11.5 });
		break;
	case Icon::Reload:
		b.M().Arc(8.0, 8.0, 6.0, 0.0, 315.0).To(14.0, 5.33).End();
		b.L({ 14.0, 2.0, 14.0, 5.33, 10.67, 5.33 });
		break;
	case Icon::Undo:
		b.M(2.75, 6.0).To(9.5, 6.0).Arc(9.5, 9.5, 3.5, 270.0, 450.0).To(7.0, 13.0).End();
		b.L({ 6.0, 2.75, 2.75, 6.0, 6.0, 9.25 });
		break;
	case Icon::Redo:
		b.M(13.25, 6.0).To(6.5, 6.0).Arc(6.5, 9.5, 3.5, 270.0, 90.0).To(9.0, 13.0).End();
		b.L({ 10.0, 2.75, 13.25, 6.0, 10.0, 9.25 });
		break;
	case Icon::Cut:
		b.C(4.5, 12.0, 2.0);
		b.C(11.5, 12.0, 2.0);
		b.L({ 5.63, 10.35, 11.0, 2.5 });
		b.L({ 10.37, 10.35, 5.0, 2.5 });
		break;
	case Icon::Copy:			TwoSheets(b); break;
	case Icon::Paste:			Clipboard(b); break;
	case Icon::Duplicate:
		TwoSheets(b);
		b.L({ 9.75, 7.75, 9.75, 11.75 });
		b.L({ 7.75, 9.75, 11.75, 9.75 });
		break;
	case Icon::Rename:
		b.Z({ 11.0, 2.5, 13.5, 5.0, 5.5, 13.0, 2.5, 13.5, 3.0, 10.5 });
		b.L({ 9.5, 4.0, 12.0, 6.5 });
		b.L({ 9.0, 13.5, 13.5, 13.5 });
		break;
	case Icon::Delete:
		b.L({ 2.5, 4.0, 13.5, 4.0 });
		b.L({ 6.0, 4.0, 6.0, 2.5, 10.0, 2.5, 10.0, 4.0 });
		b.L({ 3.75, 4.0, 4.5, 14.0, 11.5, 14.0, 12.25, 4.0 });
		b.L({ 6.75, 7.0, 6.75, 11.0 });
		b.L({ 9.25, 7.0, 9.25, 11.0 });
		break;
	case Icon::Plus:
		b.L({ 8.0, 3.0, 8.0, 13.0 });
		b.L({ 3.0, 8.0, 13.0, 8.0 });
		break;
	case Icon::Minus:			b.L({ 3.0, 8.0, 13.0, 8.0 }); break;
	case Icon::Clear:
		b.C(8.0, 8.0, 6.25);
		b.L({ 5.75, 5.75, 10.25, 10.25 });
		b.L({ 10.25, 5.75, 5.75, 10.25 });
		break;
	case Icon::Check:			b.L({ 3.0, 8.5, 6.5, 12.0, 13.0, 4.5 }); break;
	case Icon::Search:			Lens(b, 7.0, 7.0, 4.75, 14.0); break;
	case Icon::Filter:			b.Z({ 2.0, 3.0, 14.0, 3.0, 9.5, 8.5, 9.5, 13.5, 6.5, 12.0, 6.5, 8.5 }); break;
	case Icon::Sort:
		b.Arrow(5.0, 2.5, 5.0, 13.5, 3.0);
		b.Arrow(11.0, 13.5, 11.0, 2.5, 3.0);
		break;
	case Icon::SortAsc:
		b.Arrow(4.0, 2.5, 4.0, 13.5, 3.0);
		b.L({ 8.0, 4.0, 10.0, 4.0 });
		b.L({ 8.0, 8.0, 12.0, 8.0 });
		b.L({ 8.0, 12.0, 14.0, 12.0 });
		break;
	case Icon::SortDesc:
		b.Arrow(4.0, 2.5, 4.0, 13.5, 3.0);
		b.L({ 8.0, 4.0, 14.0, 4.0 });
		b.L({ 8.0, 8.0, 12.0, 8.0 });
		b.L({ 8.0, 12.0, 10.0, 12.0 });
		break;
	case Icon::Settings:
		Gear(b, 8.0, 8.0, 6.5, 4.75, 8);
		b.C(8.0, 8.0, 2.25);
		break;
	case Icon::Eye:
		EyeShape(b);
		b.D(8.0, 8.0, 2.25);
		break;
	case Icon::EyeOff:
		EyeShape(b);
		b.D(8.0, 8.0, 2.25);
		b.L({ 2.5, 2.5, 13.5, 13.5 });
		break;
	case Icon::Lock:
		b.R(3.0, 7.5, 13.0, 14.0, 1.5);
		b.M(5.25, 7.5).To(5.25, 5.0).Arc(8.0, 5.0, 2.75, 180.0, 360.0).To(10.75, 7.5).End();
		b.D(8.0, 10.75, 1.0);
		break;
	case Icon::Unlock:
		b.R(3.0, 7.5, 13.0, 14.0, 1.5);
		b.M(5.25, 7.5).To(5.25, 5.0).Arc(8.0, 5.0, 2.75, 180.0, 345.0).End();
		b.D(8.0, 10.75, 1.0);
		break;
	case Icon::Link:
		b.M(6.0, 11.33).To(4.67, 11.33).Arc(4.67, 8.0, 3.33, 90.0, 270.0).To(6.0, 4.67).End();
		b.M(10.0, 4.67).To(11.33, 4.67).Arc(11.33, 8.0, 3.33, 270.0, 450.0).To(10.0, 11.33).End();
		b.L({ 5.33, 8.0, 10.67, 8.0 });
		break;
	case Icon::Unlink:
		b.M(5.5, 11.0).To(4.25, 11.0).Arc(4.25, 8.0, 3.0, 90.0, 270.0).To(5.5, 5.0).End();
		b.M(10.5, 5.0).To(11.75, 5.0).Arc(11.75, 8.0, 3.0, 270.0, 450.0).To(10.5, 11.0).End();
		b.L({ 8.0, 2.0, 8.0, 3.5 });
		b.L({ 8.0, 12.5, 8.0, 14.0 });
		break;
	case Icon::Star:			Star(b, 8.0, 8.5, 6.75, 3.0, 5, false); break;
	case Icon::StarFilled:		Star(b, 8.0, 8.5, 6.75, 3.0, 5, true); break;
	case Icon::Bookmark:		b.Z({ 4.0, 2.0, 12.0, 2.0, 12.0, 14.0, 8.0, 10.75, 4.0, 14.0 }); break;
	case Icon::BookmarkFilled:	b.FZ({ 4.0, 2.0, 12.0, 2.0, 12.0, 14.0, 8.0, 10.75, 4.0, 14.0 }); break;
	case Icon::BookmarkPrev:
		b.Z({ 7.0, 2.5, 13.0, 2.5, 13.0, 13.5, 10.0, 11.0, 7.0, 13.5 });
		b.L({ 4.5, 5.5, 2.0, 8.0, 4.5, 10.5 });
		break;
	case Icon::BookmarkNext:
		b.Z({ 3.0, 2.5, 9.0, 2.5, 9.0, 13.5, 6.0, 11.0, 3.0, 13.5 });
		b.L({ 11.5, 5.5, 14.0, 8.0, 11.5, 10.5 });
		break;
	case Icon::History:
		b.M().Arc(8.0, 8.0, 6.0, 180.0, -135.0).To(2.0, 5.33).End();
		b.L({ 2.0, 2.0, 2.0, 5.33, 5.33, 5.33 });
		b.L({ 8.0, 4.67, 8.0, 8.0, 10.67, 9.33 });
		break;
	case Icon::Back:
		b.L({ 13.0, 8.0, 3.0, 8.0 });
		b.L({ 7.5, 3.5, 3.0, 8.0, 7.5, 12.5 });
		break;
	case Icon::Forward:
		b.L({ 3.0, 8.0, 13.0, 8.0 });
		b.L({ 8.5, 3.5, 13.0, 8.0, 8.5, 12.5 });
		break;
	// to the parent folder: a corner arrow, so it never reads as Move Up
	case Icon::Up:
		b.L({ 9.33, 6.0, 6.0, 2.67, 2.67, 6.0 });
		b.M(13.33, 13.33).To(8.67, 13.33).Arc(8.67, 10.67, 2.67, 90.0, 180.0).To(6.0, 2.67).End();
		break;
	case Icon::Home:
		b.L({ 2.0, 7.5, 8.0, 2.5, 14.0, 7.5 });
		b.L({ 3.5, 6.25, 3.5, 14.0, 12.5, 14.0, 12.5, 6.25 });
		b.L({ 6.5, 14.0, 6.5, 10.0, 9.5, 10.0, 9.5, 14.0 });
		break;
	// Show in Explorer: the folder, and a magnifier in its corner finding the file there
	case Icon::Reveal:
		b.L({ 6.75, 13.5, 1.5, 13.5, 1.5, 3.0, 6.0, 3.0, 7.5, 4.75, 14.5, 4.75, 14.5, 7.0 });
		Lens(b, 10.75, 10.75, 2.5, 14.5);
		break;
	case Icon::Info:
		b.C(8.0, 8.0, 6.25);
		b.L({ 8.0, 7.25, 8.0, 11.25 });
		b.D(8.0, 4.9, 1.0);
		break;
	case Icon::Help:
		b.C(8.0, 8.0, 6.25);
		b.M().Arc(8.0, 6.5, 2.1, 200.0, 380.0).To(8.0, 9.0).End();
		b.D(8.0, 11.5, 1.0);
		break;
	case Icon::Keyboard:
		b.R(1.5, 4.0, 14.5, 12.0, 1.5);
		for (double x : { 4.25, 6.75, 9.25, 11.75 })
			b.D(x, 6.75, 0.8);
		b.L({ 5.0, 9.5, 11.0, 9.5 });
		break;
	case Icon::ExpandAll:
		b.L({ 4.5, 6.0, 8.0, 2.5, 11.5, 6.0 });
		b.L({ 4.5, 10.0, 8.0, 13.5, 11.5, 10.0 });
		break;
	case Icon::CollapseAll:
		b.L({ 4.5, 2.5, 8.0, 6.0, 11.5, 2.5 });
		b.L({ 4.5, 13.5, 8.0, 10.0, 11.5, 13.5 });
		break;
	case Icon::MoveUp:			b.Arrow(8.0, 13.5, 8.0, 2.5, 4.5); break;
	case Icon::MoveDown:		b.Arrow(8.0, 2.5, 8.0, 13.5, 4.5); break;
	case Icon::Globe:
		b.C(8.0, 8.0, 6.25);
		b.E(8.0, 8.0, 2.75, 6.25);
		b.L({ 1.75, 8.0, 14.25, 8.0 });
		break;
	case Icon::Key:
		b.C(5.0, 11.0, 3.0);
		b.L({ 7.12, 8.88, 13.5, 2.5 });
		b.L({ 11.0, 5.0, 13.0, 7.0 });
		b.L({ 9.25, 6.75, 10.75, 8.25 });
		break;
	// the sides are arcs tangent to the straight upper sides, meeting in a point at the foot
	case Icon::Shield:
		b.M(8.0, 1.5).To(13.5, 3.5).To(13.5, 8.0).Arc(6.91, 8.0, 6.59, 0.0, 80.5).Arc(9.09, 8.0, 6.59, 99.5, 180.0)
			.To(2.5, 3.5).End(true);
		break;
	case Icon::Tag:
		b.Z({ 2.0, 2.0, 8.5, 2.0, 14.25, 7.75, 7.75, 14.25, 2.0, 8.5 });
		b.D(5.25, 5.25, 1.1);
		break;
	case Icon::Picture:
		b.R(1.5, 2.5, 14.5, 13.5, 1.5);
		b.C(5.5, 6.0, 1.5);
		b.L({ 1.5, 12.0, 6.0, 8.0, 9.0, 11.0, 11.0, 9.0, 14.5, 12.5 });
		break;
	case Icon::Terminal:
		b.R(1.5, 2.5, 14.5, 13.5, 1.5);
		b.L({ 4.5, 6.0, 6.5, 8.0, 4.5, 10.0 });
		b.L({ 8.0, 10.5, 11.0, 10.5 });
		break;
	case Icon::Refresh:
		b.M().Arc(8.0, 8.0, 6.0, 180.0, 315.0).To(14.0, 5.33).End();
		b.L({ 14.0, 2.0, 14.0, 5.33, 10.67, 5.33 });
		b.M().Arc(8.0, 8.0, 6.0, 0.0, 135.0).To(2.0, 10.67).End();
		b.L({ 5.33, 10.67, 2.0, 10.67, 2.0, 14.0 });
		break;

	// status ------------------------------------------------------------------
	case Icon::Error:
		Regular(b, 8.0, 8.0, 6.5, 8, 22.5);
		b.L({ 8.0, 4.75, 8.0, 8.75 });
		b.D(8.0, 11.0, 1.0);
		break;
	case Icon::Warning:
		b.Z({ 8.0, 2.0, 14.5, 13.5, 1.5, 13.5 });
		b.L({ 8.0, 6.25, 8.0, 9.5 });
		b.D(8.0, 11.5, 0.9);
		break;
	case Icon::Success:
		b.C(8.0, 8.0, 6.25);
		b.L({ 5.0, 8.25, 7.25, 10.5, 11.0, 6.0 });
		break;
	case Icon::Required:
		b.L({ 8.0, 2.5, 8.0, 13.5 });
		b.L({ 3.24, 5.25, 12.76, 10.75 });
		b.L({ 3.24, 10.75, 12.76, 5.25 });
		break;
	// still, the way a screenshot shows it; the kit turns it
	case Icon::Spinner:
		b.Quiet().C(8.0, 8.0, 5.5);
		b.Quiet(false).A(8.0, 8.0, 5.5, 270.0, 400.0);
		break;
	case Icon::Dirty:			b.D(8.0, 8.0, 3.25); break;

	// build and run -----------------------------------------------------------
	// the collision form is triangles
	case Icon::BuildCollision:
		b.Z({ 8.0, 2.0, 14.5, 13.5, 1.5, 13.5 });
		b.Z({ 4.75, 7.75, 11.25, 7.75, 8.0, 13.5 });
		break;
	// the AI map is a floor of cells, seen in perspective
	case Icon::BuildAIMap:
		b.Z({ 5.0, 3.0, 11.0, 3.0, 14.5, 13.0, 1.5, 13.0 });
		b.L({ 7.0, 3.0, 5.83, 13.0 });
		b.L({ 9.0, 3.0, 10.17, 13.0 });
		b.L({ 3.6, 7.0, 12.4, 7.0 });
		break;
	case Icon::BuildGameGraph:
		b.C(8.0, 3.5, 2.0);
		b.C(3.5, 12.0, 2.0);
		b.C(12.5, 12.0, 2.0);
		b.L({ 7.06, 5.27, 4.44, 10.23 });
		b.L({ 8.94, 5.27, 11.56, 10.23 });
		b.L({ 5.5, 12.0, 10.5, 12.0 });
		break;
	case Icon::BuildAll:
		b.Z({ 8.0, 2.0, 14.5, 5.25, 8.0, 8.5, 1.5, 5.25 });
		b.L({ 1.5, 8.25, 8.0, 11.5, 14.5, 8.25 });
		b.L({ 1.5, 11.25, 8.0, 14.5, 14.5, 11.25 });
		break;
	case Icon::Play:			b.FZ({ 4.5, 2.5, 13.0, 8.0, 4.5, 13.5 }); break;
	case Icon::PlayGame:
		b.R(1.5, 2.0, 14.5, 11.0, 1.5);
		b.L({ 8.0, 11.0, 8.0, 14.0 });
		b.L({ 5.0, 14.0, 11.0, 14.0 });
		b.FZ({ 6.75, 4.5, 10.25, 6.5, 6.75, 8.5 });
		break;
	case Icon::PlayClean:
		b.Z({ 3.0, 3.0, 10.5, 8.25, 3.0, 13.5 });
		b.L({ 12.5, 1.5, 12.5, 5.5 });
		b.L({ 10.5, 3.5, 14.5, 3.5 });
		break;
	case Icon::Stop:
		b.RF(3.5, 3.5, 12.5, 12.5, 1.5);
		b.R(3.5, 3.5, 12.5, 12.5, 1.5);
		break;
	case Icon::ReloadConfigs:
		PageCut(b);
		b.ArcArrow(11.5, 11.5, 2.75, 2.75, 180.0, 450.0, 2.25);
		break;
	case Icon::GameFolder:
		Folder(b);
		b.FZ({ 6.5, 7.0, 10.5, 9.25, 6.5, 11.5 });
		break;
	// a mod is a piece that fits the game: a puzzle piece
	case Icon::BuildMod:
		b.M(2.5, 4.5).To(6.33, 4.5).Arc(8.0, 3.6, 1.9, 151.7, 388.3).To(11.5, 4.5).To(11.5, 7.58)
			.Arc(12.4, 9.25, 1.9, 241.7, 478.3).To(11.5, 14.0).To(2.5, 14.0).End(true);
		break;
	case Icon::Package:
		b.R(1.5, 2.5, 14.5, 5.5, 1.0);
		b.L({ 2.5, 5.5, 2.5, 13.5, 13.5, 13.5, 13.5, 5.5 });
		b.L({ 6.5, 8.5, 9.5, 8.5 });
		break;
	case Icon::Publish:
		b.Arrow(8.0, 11.0, 8.0, 2.0, 4.5);
		b.L({ 2.0, 10.0, 2.0, 14.0, 14.0, 14.0, 14.0, 10.0 });
		break;
	case Icon::Releases:
		b.Z({ 1.5, 3.5, 6.75, 3.5, 11.5, 8.25, 7.0, 12.75, 1.5, 7.25 });
		b.D(4.25, 6.25, 1.0);
		b.L({ 9.25, 3.5, 14.0, 8.25, 9.5, 12.75 });
		break;

	// viewport ----------------------------------------------------------------
	case Icon::Select:			b.Z({ 3.0, 3.0, 13.5, 7.25, 9.0, 9.0, 7.25, 13.5 }); break;
	// dropped onto the ground
	case Icon::Place:
		b.Arrow(8.0, 1.5, 8.0, 9.0, 3.5);
		b.E(8.0, 12.0, 5.5, 2.0);
		break;
	case Icon::Move:
		b.L({ 8.0, 1.5, 8.0, 14.5 });
		b.L({ 1.5, 8.0, 14.5, 8.0 });
		b.L({ 6.0, 3.5, 8.0, 1.5, 10.0, 3.5 });
		b.L({ 6.0, 12.5, 8.0, 14.5, 10.0, 12.5 });
		b.L({ 3.5, 6.0, 1.5, 8.0, 3.5, 10.0 });
		b.L({ 12.5, 6.0, 14.5, 8.0, 12.5, 10.0 });
		break;
	// turning about a pivot: a circle that stops short of itself around a dot - the dot is what
	// keeps it apart from Reload
	case Icon::Rotate:
		b.ArcArrow(8.0, 8.0, 5.5, 5.5, 135.0, 405.0, 2.75);
		b.D(8.0, 8.0, 1.5);
		break;
	case Icon::Scale:
		b.L({ 8.0, 2.0, 2.0, 2.0, 2.0, 14.0, 14.0, 14.0, 14.0, 8.0 });
		b.L({ 9.33, 10.0, 6.0, 10.0, 6.0, 6.67 });
		b.L({ 10.67, 2.0, 14.0, 2.0, 14.0, 5.33 });
		b.L({ 14.0, 2.0, 6.0, 10.0 });
		break;
	case Icon::SnapGrid:
		b.L({ 5.5, 1.5, 5.5, 14.5 });
		b.L({ 10.5, 1.5, 10.5, 14.5 });
		b.L({ 1.5, 5.5, 14.5, 5.5 });
		b.L({ 1.5, 10.5, 14.5, 10.5 });
		break;
	case Icon::SnapAngle:
		b.L({ 14.0, 13.5, 2.0, 13.5, 10.5, 3.5 });
		b.A(2.0, 13.5, 6.5, -49.6, 0.0);
		break;
	case Icon::SnapScale:
		b.Z({ 1.5, 11.0, 11.0, 1.5, 14.5, 5.0, 5.0, 14.5 });
		b.L({ 3.88, 8.63, 5.28, 10.03 });
		b.L({ 6.25, 6.25, 7.65, 7.65 });
		b.L({ 8.63, 3.88, 10.03, 5.28 });
		break;
	case Icon::SnapObject:
		b.M(3.0, 2.5).To(3.0, 8.5).Arc(8.0, 8.5, 5.0, 180.0, 0.0).To(13.0, 2.5).To(9.5, 2.5).To(9.5, 8.5)
			.Arc(8.0, 8.5, 1.5, 0.0, 180.0).To(6.5, 2.5).End(true);
		b.L({ 3.0, 5.5, 6.5, 5.5 });
		b.L({ 9.5, 5.5, 13.0, 5.5 });
		break;
	case Icon::FrameAll:
		Corners(b);
		b.R(5.0, 5.0, 11.0, 11.0, 1.0);
		break;
	case Icon::FrameSelection:
		Corners(b);
		b.D(8.0, 8.0, 2.25);
		break;
	// the view volume: widening from the eye, or parallel
	case Icon::Perspective:
		b.Z({ 4.5, 6.0, 14.0, 2.5, 14.0, 13.5, 4.5, 10.0 });
		b.D(2.0, 8.0, 1.1);
		break;
	case Icon::Ortho:
		b.Z({ 4.5, 4.0, 14.0, 4.0, 14.0, 12.0, 4.5, 12.0 });
		b.D(2.0, 8.0, 1.1);
		break;
	case Icon::Lit:
		b.C(8.0, 8.0, 3.0);
		for (int i = 0; i < 8; ++i)
		{
			const double a = i * 45.0 * kDeg;
			b.L({ 8.0 + 5.0 * cos(a), 8.0 + 5.0 * sin(a), 8.0 + 6.75 * cos(a), 8.0 + 6.75 * sin(a) });
		}
		break;
	case Icon::Unlit:
		b.C(8.0, 8.0, 3.0);
		for (int i = 0; i < 8; ++i)
		{
			const double a = i * 45.0 * kDeg;
			b.D(8.0 + 5.75 * cos(a), 8.0 + 5.75 * sin(a), 0.9);
		}
		break;
	case Icon::Wireframe:
		b.Quiet().Z({ 5.0, 2.0, 14.0, 2.0, 14.0, 11.0, 5.0, 11.0 });
		b.L({ 2.0, 14.0, 5.0, 11.0 });
		b.Quiet(false).Z({ 2.0, 5.0, 11.0, 5.0, 11.0, 14.0, 2.0, 14.0 });
		b.L({ 2.0, 5.0, 5.0, 2.0 });
		b.L({ 11.0, 5.0, 14.0, 2.0 });
		b.L({ 11.0, 14.0, 14.0, 11.0 });
		break;
	// a bone the way a rig shows one: a joint, a long diamond, a joint
	case Icon::Bones:
		b.C(4.0, 12.0, 1.75);
		b.C(12.5, 3.5, 1.25);
		b.Z({ 5.24, 10.76, 8.18, 10.51, 11.62, 4.38, 5.5, 7.83 });
		break;
	case Icon::Stats:
		b.L({ 2.0, 2.0, 2.0, 14.0, 14.0, 14.0 });
		b.L({ 5.33, 11.33, 5.33, 9.33 });
		b.L({ 8.67, 11.33, 8.67, 3.33 });
		b.L({ 12.0, 11.33, 12.0, 6.0 });
		break;
	case Icon::ViewOptions:
		b.L({ 2.0, 4.5, 8.5, 4.5 });
		b.C(10.5, 4.5, 2.0);
		b.L({ 12.5, 4.5, 14.0, 4.5 });
		b.L({ 2.0, 11.5, 3.5, 11.5 });
		b.C(5.5, 11.5, 2.0);
		b.L({ 7.5, 11.5, 14.0, 11.5 });
		break;

	// scene object classes ----------------------------------------------------
	case Icon::SceneObject:
		b.Z({ 8.0, 1.5, 14.0, 4.75, 14.0, 11.25, 8.0, 14.5, 2.0, 11.25, 2.0, 4.75 });
		b.L({ 2.0, 4.75, 8.0, 8.0, 14.0, 4.75 });
		b.L({ 8.0, 8.0, 8.0, 14.5 });
		break;
	case Icon::Light:
		b.M(6.0, 12.0).To(4.82, 9.68).Arc(8.0, 6.5, 4.5, 135.0, 405.0).To(10.0, 12.0).End();
		b.L({ 6.0, 12.0, 10.0, 12.0 });
		b.L({ 6.5, 14.25, 9.5, 14.25 });
		break;
	case Icon::SoundSource:
		b.Z({ 2.0, 6.0, 5.0, 6.0, 9.0, 2.5, 9.0, 13.5, 5.0, 10.0, 2.0, 10.0 });
		b.A(9.0, 8.0, 3.0, -45.0, 45.0);
		b.A(9.0, 8.0, 5.5, -45.0, 45.0);
		break;
	// sound that fills a space: waves to both sides
	case Icon::SoundEnv:
		b.D(8.0, 8.0, 1.25);
		b.A(8.0, 8.0, 3.0, -50.0, 50.0);
		b.A(8.0, 8.0, 3.0, 130.0, 230.0);
		b.A(8.0, 8.0, 5.75, -50.0, 50.0);
		b.A(8.0, 8.0, 5.75, 130.0, 230.0);
		break;
	case Icon::Glow:			b.Z({ 8.0, 1.5, 9.5, 6.5, 14.5, 8.0, 9.5, 9.5, 8.0, 14.5, 6.5, 9.5, 1.5, 8.0, 6.5, 6.5 }); break;
	case Icon::Shape:
		b.C(5.5, 10.5, 3.75);
		b.R(8.0, 2.0, 14.0, 8.0, 1.0);
		break;
	case Icon::Spawn:
		b.M().Arc(8.0, 6.5, 5.0, 150.0, 390.0).To(8.0, 14.5).End(true);
		b.C(8.0, 6.5, 1.75);
		break;
	case Icon::Way:
		b.C(4.0, 12.67, 2.0);
		b.C(12.0, 3.33, 2.0);
		b.M(6.0, 12.67).To(11.67, 12.67).Arc(11.67, 10.33, 2.33, 90.0, -90.0).To(4.33, 8.0)
			.Arc(4.33, 5.67, 2.33, 90.0, 270.0).To(10.0, 3.33).End();
		break;
	case Icon::Sector:			b.Z({ 2.0, 5.0, 7.0, 2.0, 14.0, 4.0, 13.0, 12.5, 6.5, 14.0, 2.0, 11.0 }); break;
	case Icon::Portal:
		b.E(8.0, 8.0, 3.75, 6.5);
		b.Quiet().E(8.0, 8.0, 1.75, 4.25);
		break;
	case Icon::Group:
		b.Dashed().R(1.5, 1.5, 14.5, 14.5, 2.0);
		b.Dashed(false).R(4.0, 4.0, 8.5, 8.5, 1.0);
		b.R(7.5, 7.5, 12.0, 12.0, 1.0);
		break;
	case Icon::Particles:
		b.D(4.0, 11.0, 1.5);
		b.D(8.0, 5.0, 1.1);
		b.D(12.0, 9.0, 1.3);
		b.D(10.5, 13.5, 0.9);
		b.D(3.5, 4.0, 0.9);
		b.D(13.0, 3.0, 0.8);
		break;
	case Icon::DetailObjects:
		b.L({ 1.5, 14.0, 14.5, 14.0 });
		b.L({ 4.0, 14.0, 2.5, 7.0 });
		b.L({ 6.5, 14.0, 6.5, 4.0 });
		b.L({ 9.0, 14.0, 10.5, 6.5 });
		b.L({ 11.5, 14.0, 13.5, 9.0 });
		break;
	// nodes on a lattice
	case Icon::AIMap:
		b.Quiet();
		for (double v : { 3.5, 8.0, 12.5 })
		{
			b.L({ 3.5, v, 12.5, v });
			b.L({ v, 3.5, v, 12.5 });
		}
		b.Quiet(false);
		for (double y : { 3.5, 8.0, 12.5 })
			for (double x : { 3.5, 8.0, 12.5 })
				b.D(x, y, 1.2);
		break;
	// a splat on a wall
	case Icon::Wallmark:
		b.R(1.5, 1.5, 14.5, 14.5, 1.5);
		Star(b, 8.0, 8.0, 4.5, 2.25, 7, false);
		break;
	case Icon::FogVolume:
		Cloud(b);
		b.L({ 3.0, 12.5, 13.0, 12.5 });
		b.L({ 5.5, 14.5, 10.5, 14.5 });
		break;

	// files -------------------------------------------------------------------
	case Icon::Folder:			Folder(b); break;
	case Icon::FolderOpen:
		b.L({ 1.5, 13.5, 1.5, 3.0, 6.0, 3.0, 7.5, 4.75, 13.0, 4.75, 13.0, 7.0 });
		b.Z({ 1.5, 13.5, 4.0, 7.0, 14.5, 7.0, 12.0, 13.5 });
		break;
	case Icon::File:			Page(b); break;
	case Icon::FileTexture:
		Page(b);
		b.D(6.5, 7.5, 1.0);
		b.L({ 5.0, 12.5, 7.0, 10.0, 8.5, 11.5, 9.5, 10.5, 11.0, 12.5 });
		break;
	case Icon::FileModel:
		Page(b);
		b.Z({ 8.0, 7.0, 10.5, 8.25, 10.5, 11.25, 8.0, 12.5, 5.5, 11.25, 5.5, 8.25 });
		b.L({ 5.5, 8.25, 8.0, 9.5, 10.5, 8.25 });
		b.L({ 8.0, 9.5, 8.0, 12.5 });
		break;
	case Icon::FileSound:
		Page(b);
		b.C(6.75, 11.75, 1.25);
		b.L({ 8.0, 11.75, 8.0, 7.0, 10.25, 8.0 });
		break;
	case Icon::FileScript:
		Page(b);
		b.L({ 7.0, 7.25, 6.25, 7.25, 5.75, 7.75, 5.75, 9.25, 5.0, 9.75, 5.75, 10.25, 5.75, 11.75, 6.25, 12.25, 7.0, 12.25 });
		b.L({ 9.0, 7.25, 9.75, 7.25, 10.25, 7.75, 10.25, 9.25, 11.0, 9.75, 10.25, 10.25, 10.25, 11.75, 9.75, 12.25, 9.0, 12.25 });
		break;
	// keys and values: sliders, not lines of text
	case Icon::FileConfig:
		Page(b);
		b.L({ 5.5, 8.5, 10.5, 8.5 });
		b.D(9.0, 8.5, 1.1);
		b.L({ 5.5, 11.75, 10.5, 11.75 });
		b.D(7.0, 11.75, 1.1);
		break;
	case Icon::FileXml:
		Page(b);
		b.L({ 6.5, 8.0, 5.0, 9.75, 6.5, 11.5 });
		b.L({ 9.5, 8.0, 11.0, 9.75, 9.5, 11.5 });
		b.L({ 8.75, 7.5, 7.25, 12.0 });
		break;
	case Icon::FileText:
		Page(b);
		b.L({ 5.5, 8.0, 10.5, 8.0 });
		b.L({ 5.5, 10.25, 10.5, 10.25 });
		b.L({ 5.5, 12.5, 8.5, 12.5 });
		break;
	case Icon::FileArchive:
		Page(b);
		b.L({ 6.75, 3.0, 6.75, 4.0 });
		b.L({ 6.75, 5.5, 6.75, 6.5 });
		b.R(5.75, 8.25, 7.75, 11.25, 1.0);
		break;
	case Icon::Level:
		b.Z({ 1.5, 3.5, 5.5, 2.0, 10.5, 3.5, 14.5, 2.0, 14.5, 12.5, 10.5, 14.0, 5.5, 12.5, 1.5, 14.0 });
		b.L({ 5.5, 2.0, 5.5, 12.5 });
		b.L({ 10.5, 3.5, 10.5, 14.0 });
		break;
	// a quest is a flow: one node splitting into two
	case Icon::Quest:
		b.R(U(0.28), U(0.03), U(0.72), U(0.23), 0.77);
		b.R(U(0.02), U(0.63), U(0.44), U(0.83), 0.77);
		b.R(U(0.56), U(0.63), U(0.98), U(0.83), 0.77);
		b.Quiet().L({ U(0.50), U(0.23), U(0.50), U(0.44) });
		b.L({ U(0.23), U(0.63), U(0.23), U(0.44), U(0.77), U(0.44), U(0.77), U(0.63) });
		break;
	// a behaviour graph moves people: a route with its waypoints and an arrow at the end
	case Icon::Behaviour:
		b.L({ U(0.07), U(0.82), U(0.30), U(0.44), U(0.55), U(0.63), U(0.753), U(0.286) });
		// fatter than the line they sit on, or they read as kinks and not as places
		b.D(U(0.07), U(0.82), 1.3);
		b.D(U(0.30), U(0.44), 1.3);
		b.D(U(0.55), U(0.63), 1.3);
		b.F({ U(0.86), U(0.12), U(0.826), U(0.330), U(0.672), U(0.242) });
		break;
	// a person: the head and the shoulders. The template is the same person dashed, because it
	// never becomes anybody in the game.
	case Icon::Character:
		b.C(U(0.5), U(0.3), 0.185 * 14.0);
		b.A(U(0.5), U(0.98), 0.36 * 14.0, 180.0, 360.0);
		break;
	case Icon::CharacterTemplate:
		b.Dashed().A(U(0.5), U(0.3), 0.185 * 14.0, 0.0, 360.0);
		b.A(U(0.5), U(0.98), 0.36 * 14.0, 180.0, 360.0);
		break;
	// screens you switch between: the device's screen, a card on it, the tab bar at its foot
	case Icon::PdaInterface:
		b.R(U(0.04), U(0.14), U(0.96), U(0.86), 0.10 * 14.0);
		b.R(U(0.20), U(0.30), U(0.80), U(0.52), 0.04 * 14.0);
		b.Quiet().L({ U(0.04), U(0.64), U(0.96), U(0.64) });
		b.Quiet(false);
		for (double x : { 0.27, 0.50, 0.73 })
			b.D(U(x), U(0.75), 0.055 * 14.0);
		break;

	// graph editor ------------------------------------------------------------
	case Icon::Flow:
		b.R(2.0, 2.0, 7.0, 7.0, 1.0);
		b.R(9.0, 9.0, 14.0, 14.0, 1.0);
		b.L({ 4.5, 7.0, 4.5, 11.5, 9.0, 11.5 });
		break;
	case Icon::Wait:
		b.L({ 3.5, 1.5, 12.5, 1.5 });
		b.L({ 3.5, 14.5, 12.5, 14.5 });
		b.L({ 4.5, 1.5, 4.5, 4.0, 8.0, 8.0, 4.5, 12.0, 4.5, 14.5 });
		b.L({ 11.5, 1.5, 11.5, 4.0, 8.0, 8.0, 11.5, 12.0, 11.5, 14.5 });
		break;
	case Icon::Condition:		b.Z({ 8.0, 1.5, 14.5, 8.0, 8.0, 14.5, 1.5, 8.0 }); break;
	case Icon::Variable:
		b.A(7.5, 8.0, 6.0, 128.0, 232.0);
		b.A(8.5, 8.0, 6.0, -52.0, 52.0);
		b.L({ 5.75, 5.75, 10.25, 10.25 });
		b.L({ 10.25, 5.75, 5.75, 10.25 });
		break;
	case Icon::SpawnNpc:
		Bust(b, 6.0, 4.67, 2.67, 10.0, 4.5, 14.0);
		b.L({ 12.5, 5.25, 12.5, 9.25 });
		b.L({ 10.5, 7.25, 14.5, 7.25 });
		break;
	case Icon::Dialog:			b.Z({ 2.0, 2.5, 14.0, 2.5, 14.0, 11.0, 6.0, 11.0, 2.0, 14.0 }); break;
	// an item is loot: a gem
	case Icon::Item:
		b.Z({ 4.0, 2.25, 12.0, 2.25, 14.5, 6.0, 8.0, 14.25, 1.5, 6.0 });
		b.L({ 7.33, 2.25, 5.5, 6.0, 8.0, 14.25, 10.5, 6.0, 8.67, 2.25 });
		b.L({ 1.5, 6.0, 14.5, 6.0 });
		break;
	case Icon::PdaMsg:
		b.R(1.5, 3.0, 14.5, 13.0, 1.5);
		b.L({ 1.5, 4.75, 8.0, 9.0, 14.5, 4.75 });
		break;
	case Icon::World:
		b.Z({ 1.5, 13.5, 6.0, 6.0, 8.5, 10.0, 10.5, 7.5, 14.5, 13.5 });
		b.D(11.75, 3.5, 1.5);
		break;
	case Icon::Relation:
		b.L({ 2.0, 5.0, 14.0, 5.0 });
		b.L({ 11.0, 2.0, 14.0, 5.0, 11.0, 8.0 });
		b.L({ 14.0, 11.0, 2.0, 11.0 });
		b.L({ 5.0, 8.0, 2.0, 11.0, 5.0, 14.0 });
		break;
	case Icon::Squad:
		Bust(b, 8.0, 4.25, 2.25, 8.5, 3.25, 14.0);
		b.C(3.0, 6.0, 1.5);
		b.M(1.5, 13.0).To(1.5, 11.0).Arc(3.0, 11.0, 1.5, 180.0, 270.0).To(3.75, 9.5).End();
		b.C(13.0, 6.0, 1.5);
		b.M(14.5, 13.0).To(14.5, 11.0).Arc(13.0, 11.0, 1.5, 360.0, 270.0).To(12.25, 9.5).End();
		break;
	// an order sends somebody somewhere
	case Icon::Order:			b.Z({ 2.0, 7.5, 14.0, 2.0, 8.5, 14.0, 7.0, 9.0 }); break;
	case Icon::Timer:
		b.C(8.0, 9.0, 5.5);
		b.L({ 8.0, 9.0, 8.0, 6.0 });
		b.L({ 6.5, 1.75, 9.5, 1.75 });
		b.L({ 8.0, 1.75, 8.0, 3.5 });
		break;
	case Icon::Trigger:			b.Z({ 9.0, 1.5, 3.0, 9.0, 8.0, 9.0, 7.0, 14.5, 13.0, 7.0, 8.0, 7.0 }); break;
	// the final state of a state chart
	case Icon::End:
		b.C(8.0, 8.0, 6.0);
		b.D(8.0, 8.0, 3.0);
		break;
	case Icon::Task:
		b.R(2.0, 2.0, 14.0, 14.0, 2.0);
		b.L({ 5.0, 8.0, 7.25, 10.25, 11.0, 6.0 });
		break;
	// a role is a part somebody is cast in: an ID card
	case Icon::Role:
		b.R(1.5, 3.0, 14.5, 13.0, 1.5);
		b.C(5.5, 7.0, 1.5);
		b.A(5.5, 11.5, 2.5, 180.0, 360.0);
		b.L({ 9.5, 6.5, 12.5, 6.5 });
		b.L({ 9.5, 9.5, 12.5, 9.5 });
		break;
	case Icon::Minimap:
		b.R(1.5, 2.5, 14.5, 13.5, 1.5);
		b.R(8.0, 7.0, 12.5, 11.5, 0.5);
		break;
	case Icon::AutoLayout:
		b.R(2.0, 2.0, 7.0, 7.0, 1.0);
		b.R(9.0, 2.0, 14.0, 7.0, 1.0);
		b.R(2.0, 9.0, 7.0, 14.0, 1.0);
		b.R(9.0, 9.0, 14.0, 14.0, 1.0);
		break;
	case Icon::Validate:
		Clipboard(b);
		b.L({ 5.5, 9.5, 7.25, 11.25, 10.5, 7.75 });
		break;
	case Icon::FindInProject:
		PageCut(b);
		Lens(b, 10.5, 10.75, 2.5, 14.25);
		break;
	case Icon::ZoomIn:
		Lens(b, 7.0, 7.0, 5.0, 14.0);
		b.L({ 7.0, 4.75, 7.0, 9.25 });
		b.L({ 4.75, 7.0, 9.25, 7.0 });
		break;
	case Icon::ZoomOut:
		Lens(b, 7.0, 7.0, 5.0, 14.0);
		b.L({ 4.75, 7.0, 9.25, 7.0 });
		break;
	case Icon::ZoomFit:
		Corners(b);
		Lens(b, 7.5, 7.5, 2.5, 11.0);
		break;
	// "1:1"
	case Icon::Zoom100:
		b.L({ 3.0, 5.0, 4.75, 3.5, 4.75, 12.5 });
		b.D(8.0, 6.5, 0.9);
		b.D(8.0, 10.5, 0.9);
		b.L({ 9.75, 5.0, 11.5, 3.5, 11.5, 12.5 });
		break;

	// other -------------------------------------------------------------------
	case Icon::Person:
		b.C(8.0, 8.0, 6.5);
		b.C(8.0, 6.67, 2.0);
		b.M(4.67, 13.77).To(4.67, 12.67).Arc(6.0, 12.67, 1.33, 180.0, 270.0).To(10.0, 11.33)
			.Arc(10.0, 12.67, 1.33, 270.0, 360.0).To(11.33, 13.77).End();
		break;
	// a faction is a banner
	case Icon::Community:
		b.L({ 3.0, 14.5, 3.0, 2.0 });
		b.L({ 3.0, 2.5, 13.5, 2.5, 11.0, 5.5, 13.5, 8.5, 3.0, 8.5 });
		break;
	case Icon::Rank:
		b.L({ 3.0, 6.0, 8.0, 2.5, 13.0, 6.0 });
		b.L({ 3.0, 10.0, 8.0, 6.5, 13.0, 10.0 });
		b.L({ 3.0, 14.0, 8.0, 10.5, 13.0, 14.0 });
		break;
	case Icon::Money:
		b.R(1.5, 4.0, 14.5, 12.0, 1.5);
		b.C(8.0, 8.0, 2.0);
		b.D(4.25, 8.0, 0.8);
		b.D(11.75, 8.0, 0.8);
		break;
	// a starting kit is a backpack
	case Icon::Kit:
		b.R(2.75, 4.0, 13.25, 14.5, 2.5);
		b.M(6.0, 4.0).To(6.0, 3.0).Arc(7.0, 3.0, 1.0, 180.0, 270.0).To(9.0, 2.0).Arc(9.0, 3.0, 1.0, 270.0, 360.0)
			.To(10.0, 4.0).End();
		b.L({ 5.5, 7.0, 10.5, 7.0 });
		b.L({ 5.5, 14.5, 5.5, 10.5, 10.5, 10.5, 10.5, 14.5 });
		break;
	case Icon::Area:
		b.Dashed().Z({ 2.0, 5.0, 6.5, 2.0, 14.0, 3.5, 13.5, 12.5, 5.0, 14.0, 2.0, 10.5 });
		b.Dashed(false).D(8.0, 8.0, 1.5);
		break;
	// a level changer: through a door
	case Icon::Transition:
		b.M(10.0, 2.0).To(12.5, 2.0).Arc(12.5, 3.5, 1.5, 270.0, 360.0).To(14.0, 12.5).Arc(12.5, 12.5, 1.5, 0.0, 90.0)
			.To(10.0, 14.0).End();
		b.L({ 2.0, 8.0, 10.0, 8.0 });
		b.L({ 7.0, 5.0, 10.0, 8.0, 7.0, 11.0 });
		break;
	case Icon::PlayerStart:
		Bust(b, 8.0, 3.75, 2.0, 7.5, 3.5, 12.5);
		b.Quiet().E(8.0, 12.5, 6.0, 2.0);
		break;
	// a game mode is a way to play: a gamepad
	case Icon::Mode:
		b.R(1.5, 4.5, 14.5, 12.5, 3.5);
		b.L({ 4.0, 8.5, 7.0, 8.5 });
		b.L({ 5.5, 7.0, 5.5, 10.0 });
		b.D(10.5, 7.5, 0.9);
		b.D(12.0, 9.5, 0.9);
		break;
	case Icon::Relations:
		b.C(6.0, 4.67, 2.67);
		b.M(10.67, 14.0).To(10.67, 12.67).Arc(8.0, 12.67, 2.67, 0.0, -90.0).To(4.17, 10.0)
			.Arc(4.17, 12.67, 2.67, 270.0, 180.0).To(1.5, 14.0).End();
		b.A(10.0, 4.67, 2.67, -76.0, 76.0);
		b.M(14.5, 14.0).To(14.5, 12.67).Arc(12.5, 12.67, 2.0, 0.0, -80.0).End();
		break;
	case Icon::Exclude:
		b.C(8.0, 8.0, 6.25);
		b.L({ 3.58, 3.58, 12.42, 12.42 });
		break;
	case Icon::Aspect:
		b.R(1.5, 3.0, 14.5, 13.0, 1.5);
		b.L({ 4.0, 7.5, 4.0, 5.5, 6.5, 5.5 });
		b.L({ 12.0, 8.5, 12.0, 10.5, 9.5, 10.5 });
		break;
	case Icon::Language:
		b.L({ 3.33, 5.33, 7.33, 9.33 });
		b.L({ 2.67, 9.33, 6.67, 5.33, 8.0, 3.33 });
		b.L({ 1.5, 3.33, 9.33, 3.33 });
		b.L({ 4.67, 1.5, 5.33, 1.5 });
		b.L({ 14.5, 14.5, 11.33, 8.0, 8.0, 14.5 });
		b.L({ 9.33, 12.0, 13.33, 12.0 });
		break;
	case Icon::Tasks:
		b.L({ 2.0, 4.67, 3.33, 6.0, 6.0, 3.33 });
		b.L({ 2.0, 11.33, 3.33, 12.67, 6.0, 10.0 });
		b.L({ 8.67, 4.0, 14.0, 4.0 });
		b.L({ 8.67, 8.0, 14.0, 8.0 });
		b.L({ 8.67, 12.0, 14.0, 12.0 });
		break;
	// the agents' connection: a plug
	case Icon::Mcp:
		b.L({ 5.5, 1.5, 5.5, 4.5 });
		b.L({ 10.5, 1.5, 10.5, 4.5 });
		b.M(3.5, 4.5).To(12.5, 4.5).To(12.5, 7.0).Arc(8.0, 7.0, 4.5, 0.0, 180.0).End(true);
		b.L({ 8.0, 11.5, 8.0, 14.5 });
		break;
	case Icon::Log:
		b.L({ 2.0, 3.0, 14.0, 3.0 });
		b.L({ 2.0, 6.33, 11.0, 6.33 });
		b.L({ 2.0, 9.67, 14.0, 9.67 });
		b.L({ 2.0, 13.0, 8.0, 13.0 });
		break;
	case Icon::Problems:
		b.Z({ 4.5, 2.0, 7.5, 7.5, 1.5, 7.5 });
		b.L({ 10.0, 4.75, 14.5, 4.75 });
		b.L({ 1.5, 10.75, 14.5, 10.75 });
		b.L({ 1.5, 14.0, 10.0, 14.0 });
		break;
	case Icon::Drawer:
		b.R(1.5, 2.5, 14.5, 13.5, 1.5);
		b.L({ 1.5, 10.5, 14.5, 10.5 });
		b.L({ 5.5, 7.5, 8.0, 5.0, 10.5, 7.5 });
		break;

	default:
		break;
	}
}

void EnsureBuilt()
{
	if (s_Built)
		return;
	s_Built = true;
	for (int id = 1; id < (int)Icon::Count; ++id)
	{
		B b(s_Glyphs[id]);
		Build((Icon)id, b);
	}
}

//------------------------------------------------------------------------------
// drawing
//------------------------------------------------------------------------------
struct SView
{
	ImVec2	o;
	float	u		= 1.f;
	bool	snap	= false;
	bool	odd		= false;	// an odd stroke is crisp on a pixel's centre, an even one on its edge

	ImVec2 At(const ImVec2& p, bool free = false) const
	{
		const ImVec2 s(o.x + p.x * u, o.y + p.y * u);
		if (!snap || free)
			return s;
		return odd ? ImVec2(floorf(s.x) + 0.5f, floorf(s.y) + 0.5f) : ImVec2(floorf(s.x + 0.5f), floorf(s.y + 0.5f));
	}
};

// Dashes are measured ALONG the stroke and each is flushed as ONE polyline: an arc arrives
// as many short pieces, and "every other piece" would be noise that gets finer with size.
// The pattern is stretched to fit a whole number of dashes: a loop closes on a gap, an open
// path starts and ends on a dash - otherwise a stub or a double-length dash lands at the seam.
void StrokeDashed(ImDrawList* dl, const ImVec2* pts, int n, ImU32 col, float t, float unit, bool loop)
{
	auto dist = [](const ImVec2& a, const ImVec2& b) { return sqrtf((b.x - a.x) * (b.x - a.x) + (b.y - a.y) * (b.y - a.y)); };
	float length = 0.f;
	for (int p = 0; p + 1 < n; ++p)
		length += dist(pts[p], pts[p + 1]);
	const float base_dash = 2.4f * unit, base_gap = 1.35f * unit, base_period = base_dash + base_gap;
	if (length < base_gap)
		return;
	const float count	= std::max(loop ? 3.f : 2.f, roundf((loop ? length : length + base_gap) / base_period));
	const float fit		= length / (loop ? count * base_period : count * base_period - base_gap);
	const float dash	= base_dash * fit, period = base_period * fit;
	float phase = 0.f;
	ImVector<ImVec2>& run = s_Run;
	run.resize(0);
	for (int p = 0; p + 1 < n; ++p)
	{
		const ImVec2& a = pts[p];
		const ImVec2& b = pts[p + 1];
		const float len = dist(a, b);
		if (len < 1e-4f)
			continue;
		for (float done = 0.f; done < len;)
		{
			const bool	on		= phase < dash;
			const float	take	= std::min((on ? dash : period) - phase, len - done);
			if (on)
			{
				const float u0 = done / len, u1 = (done + take) / len;
				if (run.empty())
					run.push_back(ImVec2(a.x + (b.x - a.x) * u0, a.y + (b.y - a.y) * u0));
				run.push_back(ImVec2(a.x + (b.x - a.x) * u1, a.y + (b.y - a.y) * u1));
			}
			done	+= take;
			phase	+= take;
			if (!run.empty() && phase >= dash - 1e-4f)
			{
				dl->AddPolyline(run.Data, run.Size, col, ImDrawFlags_None, t);
				run.resize(0);
			}
			if (phase >= period - 1e-4f)
				phase = 0.f;
		}
	}
	if (!run.empty())
		dl->AddPolyline(run.Data, run.Size, col, ImDrawFlags_None, t);
}

// Snapping moves a corner by up to half a pixel, and the first points of a curve that starts there
// can then lie behind it: the line doubles back over less than a pixel, and ImGui mitres that into
// a spike. Such a point goes - the one on the shorter leg, so a snapped corner outlives the curve
// points around it; an open path keeps its ends. Returns the points left.
int DropBacktracks(ImVec2* s, int n, bool closed)
{
	auto doubles_back = [](const ImVec2& a, const ImVec2& b, const ImVec2& c, float& ab, float& bc)
	{
		const float ux = b.x - a.x, uy = b.y - a.y, vx = c.x - b.x, vy = c.y - b.y;
		ab = ux * ux + uy * uy;
		bc = vx * vx + vy * vy;
		return bc < 1e-6f || (ux * vx + uy * vy < 0.f && (ab < 1.f || bc < 1.f));
	};
	float ab = 0.f, bc = 0.f;
	int w = 0;
	for (int i = 0; i < n; ++i)
	{
		s[w++] = s[i];
		while (w >= 3 && doubles_back(s[w - 3], s[w - 2], s[w - 1], ab, bc))
		{
			if (bc > ab || (!closed && i + 1 == n))
				s[w - 2] = s[w - 1];
			--w;
		}
	}
	if (closed)
	{
		while (w >= 4 && doubles_back(s[w - 2], s[w - 1], s[0], ab, bc))
			--w;
		if (w >= 4 && doubles_back(s[w - 1], s[0], s[1], ab, bc))
		{
			memmove(s, s + 1, sizeof(ImVec2) * (w - 1));
			--w;
		}
	}
	return w;
}

// Round joins and caps: the line is cut at its corners, the pieces are butted and every corner
// and end gets a disc. ImGui mitres a polyline, and a mitre at an acute corner is a spike that
// grows with the stroke. Only for an opaque colour - a translucent disc would show on the line.
void StrokeRound(ImDrawList* dl, const ImVec2* s, const u8* fl, int n, bool closed, ImU32 col, float t)
{
	int start = 0;
	if (closed)
	{
		start = -1;
		for (int i = 0; i < n && start < 0; ++i)
			if (fl[i] & pfJoin)
				start = i;
		if (start < 0)
		{
			dl->AddPolyline(s, n, col, ImDrawFlags_Closed, t);
			return;
		}
	}
	const int segments = closed ? n : n - 1;
	ImVector<ImVec2>& run = s_Run;
	run.resize(0);
	run.push_back(s[start]);
	for (int k = 1; k <= segments; ++k)
	{
		const int i = (start + k) % n;
		run.push_back(s[i]);
		if ((fl[i] & pfJoin) || k == segments)
		{
			dl->AddPolyline(run.Data, run.Size, col, ImDrawFlags_None, t);
			run.resize(0);
			run.push_back(s[i]);
		}
	}
	for (int i = 0; i < n; ++i)
		if (fl[i] & pfJoin)
			dl->AddCircleFilled(s[i], t * 0.5f, col);
}

void DrawPx(ImDrawList* dl, Icon icon, const ImVec2& o, float side, ImU32 col)
{
	const int id = (int)icon;
	const u32 alpha = (col >> IM_COL32_A_SHIFT) & 0xFF;
	if (!dl || id <= 0 || id >= (int)Icon::Count || side < 2.f || !alpha)
		return;
	EnsureBuilt();
	const SGlyph& g = s_Glyphs[id];

	SView v;
	v.o		= o;
	v.u		= side / kGrid;
	v.snap	= side <= kSnapUpTo;
	float t = kStroke * v.u;
	// small: a whole number of pixels, rounded a little down so 16 px keeps a 1 px line
	if (v.snap)
		t = floorf(t + 0.4f);
	t = std::max(t, 1.f);
	v.odd = v.snap && ((int)t & 1);

	const ImU32	quiet	= (col & ~IM_COL32_A_MASK) | ((ImU32)(alpha * kQuiet) << IM_COL32_A_SHIFT);
	const bool	round	= t > kRoundAbove && alpha == 0xFF;

	for (const SPrim& p : g.prims)
	{
		const bool	q = (p.flags & kQuietP) != 0;
		const ImU32	c = q ? quiet : col;
		switch (p.type)
		{
		case EPrim::Stroke:
		{
			const ImVec2*	src	= &g.pts[p.first];
			const u8*		fl	= &g.pflags[p.first];
			const int		n	= (int)p.count;
			ImVector<ImVec2>& s = s_Screen;
			s.resize(n);
			for (int i = 0; i < n; ++i)
				s[i] = v.At(src[i], (fl[i] & pfFree) != 0);
			const bool closed = (p.flags & kClosed) != 0;
			// a snapped glyph is at most 20 px, where no stroke is drawn round: the flags need not
			// follow the points that go
			const bool dashed = (p.flags & kDashed) != 0;
			// a dashed loop already ends on its first point
			const int kept = v.snap ? DropBacktracks(s.Data, n, closed && !dashed) : n;
			if (kept < 2)
				break;
			if (dashed)
				StrokeDashed(dl, s.Data, kept, c, t, v.u, closed);
			else if (round && !q && !v.snap)
				StrokeRound(dl, s.Data, fl, n, closed, c, t);
			else
				dl->AddPolyline(s.Data, kept, c, closed ? ImDrawFlags_Closed : ImDrawFlags_None, t);
			break;
		}
		case EPrim::Rect:
		{
			const ImVec2 a = v.At(p.a), b = v.At(p.b);
			if (p.flags & kFilled)
				dl->AddRectFilled(a, b, c, p.r * v.u);
			else
			{
				// PathRect, not AddRect: AddRect insets half a pixel, and these corners are snapped already
				dl->PathRect(a, b, p.r * v.u);
				dl->PathStroke(c, ImDrawFlags_Closed, t);
			}
			break;
		}
		case EPrim::Circle:
			if (p.flags & kFilled)
				dl->AddCircleFilled(v.At(p.a), p.r * v.u, c);
			else
				dl->AddCircle(v.At(p.a), p.r * v.u, c, 0, t);
			break;
		case EPrim::Fill:
		{
			const ImVec2* src = &g.pts[p.first];
			const int n = (int)p.count;
			ImVector<ImVec2>& s = s_Screen;
			s.resize(n);
			for (int i = 0; i < n; ++i)
				s[i] = v.At(src[i]);
			if (p.flags & kConvex)
				dl->AddConvexPolyFilled(s.Data, n, c);
			else
				dl->AddConcavePolyFilled(s.Data, n, c);
			break;
		}
		}
	}
}

const char* const kNames[] =
{
	"none", "minimize", "maximize", "restore", "close", "chevron_up", "chevron_down", "chevron_left",
	"chevron_right", "caret", "more", "overflow", "menu", "dot", "grip", "pin", "external", "sidebar_left",
	"sidebar_right", "panel_bottom", "new", "open", "save", "save_all", "reload", "undo", "redo", "cut", "copy",
	"paste", "duplicate", "rename", "delete", "plus", "minus", "clear", "check", "search", "filter", "sort",
	"sort_asc", "sort_desc", "settings", "eye", "eye_off", "lock", "unlock", "link", "unlink", "star",
	"star_filled", "bookmark", "bookmark_filled", "bookmark_prev", "bookmark_next", "history", "back",
	"forward", "up", "home", "reveal", "info", "help", "keyboard", "expand_all", "collapse_all", "move_up",
	"move_down", "globe", "key", "shield", "tag", "picture", "terminal", "refresh", "error", "warning",
	"success", "required", "spinner", "dirty", "build_collision", "build_ai_map", "build_game_graph",
	"build_all", "play", "play_game", "play_clean", "stop", "reload_configs", "game_folder", "build_mod",
	"package", "publish", "releases", "select", "place", "move", "rotate", "scale", "snap_grid", "snap_angle",
	"snap_scale", "snap_object", "frame_all", "frame_selection", "perspective", "ortho", "lit", "unlit",
	"wireframe", "bones", "stats", "view_options", "scene_object", "light", "sound_source", "sound_env", "glow",
	"shape", "spawn", "way", "sector", "portal", "group", "particles", "detail_objects", "ai_map", "wallmark",
	"fog_volume", "folder", "folder_open", "file", "file_texture", "file_model", "file_sound", "file_script",
	"file_config", "file_xml", "file_text", "file_archive", "level", "quest", "behaviour", "character",
	"character_template", "pda_interface", "flow", "wait", "condition", "variable", "spawn_npc", "dialog",
	"item", "pda_msg", "world", "relation", "squad", "order", "timer", "trigger", "end", "task", "role",
	"minimap", "auto_layout", "validate", "find_in_project", "zoom_in", "zoom_out", "zoom_fit", "zoom_100",
	"person", "community", "rank", "money", "kit", "area", "transition", "player_start", "mode", "relations",
	"exclude", "aspect", "language", "tasks", "mcp", "log", "problems", "drawer",
};
static_assert(sizeof(kNames) / sizeof(kNames[0]) == (size_t)Icon::Count, "one name per icon, in enum order");
} // namespace

void XUI::DrawIcon(ImDrawList* draw_list, Icon icon, const ImVec2& min, float size_pt, ImU32 color)
{
	DrawPx(draw_list, icon, min, size_pt * Dpi(), color);
}

void XUI::DrawIconRect(ImDrawList* draw_list, Icon icon, const ImVec2& min, const ImVec2& max, ImU32 color)
{
	const float w = max.x - min.x, h = max.y - min.y, side = w < h ? w : h;
	DrawPx(draw_list, icon, ImVec2(min.x + (w - side) * 0.5f, min.y + (h - side) * 0.5f), side, color);
}

const char* XUI::IconName(Icon icon)
{
	const int id = (int)icon;
	return id >= 0 && id < (int)Icon::Count ? kNames[id] : "";
}
