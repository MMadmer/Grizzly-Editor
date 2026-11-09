#pragma once

//------------------------------------------------------------------------------
// View > Game Lighting: the scene lit the way the linked game lights the level at
// a chosen hour and weather.
//
// The weather comes from the game's own environment\weathers\<name>.ltx (the two
// sections around the hour, blended), scaled by the game's r2_sun_lumscale*;
// the exposure is what the game's tonemap gives a dark scene (r2_tonemap_*), both
// read from the linked game's user.ltx. The level editor hands over, before every
// scene pass, the lights the game would draw near the camera - hanging lamps with
// their ambient lights, burning campfires, the dynamic level lights - and a roof
// map that tells indoors from outdoors. Bind() puts it all in two float textures
// the scene shaders read (gamedata\shaders\editor\common.h, glit_light);
// Unbind() takes them away after the pass, so previews and thumbnails keep their
// own light rig.
//------------------------------------------------------------------------------

struct ECORE_API SGameLightSource
{
	Fvector		position;
	float		range;
	Fvector		color;			// rgb x brightness, what the game feeds its light
	float		spot_cos;		// cos of a spot's half angle, 0 = point light
	Fvector		direction;		// a spot's axis
	LPCSTR		kind;			// "lamp", "lamp_ambient", "campfire", "level"
	shared_str	owner;			// the scene object it belongs to
};

class ECORE_API EGameLighting
{
public:
	enum
	{
		MAX_LIGHTS	= 128,		// nearest to the camera first
		HEADER		= 8,		// texels before the first light
		ROOF_CELLS	= 256,		// the roof map is ROOF_CELLS^2 cells of ROOF_CELL m
	};
	static constexpr float ROOF_CELL = 1.f;

	bool			enabled		= false;

	// the weather and the hour (18.5 = 18:30)
	xr_string		weather;
	float			hour		= 22.f;
	// 0 = the tonemap's: adapted to the last viewport frame, as the game's eye adapts; else this factor
	float			exposure	= 0.f;
	// what the adaptation settled on, and the frame's mean luminance it came from (0 = not measured yet)
	float			auto_exposure = 0.f;
	float			scene_lum	= 0.f;
	// the share of the hemisphere light that reaches indoors (the game bakes it per surface)
	float			indoor_hemi	= 0.15f;

	// the weather at the hour, as loaded (not yet scaled)
	Fvector			ambient, hemi, sun_color, sun_dir, fog_color;
	bool			weather_ok	= false;
	xr_string		weather_error;

	// the linked game's render settings
	float			lumscale_sun = 1.f, lumscale_hemi = 1.f, lumscale_amb = 1.f;
	bool			tonemap = true;
	float			tonemap_middlegray = 1.f, tonemap_lowlum = 0.0001f, tonemap_amount = 0.7f;

	// filled by the level editor before every scene pass
	xr_vector<SGameLightSource>	lights;
	u32				lights_in_view = 0;		// before the MAX_LIGHTS cut

	// the roof map: the height of the topmost surface over each cell
	Fvector2		roof_origin;
	u32				roof_cells	= 0;
	xr_vector<float> roof;
	bool			roof_changed = false;
	u32				roof_bake_ms = 0;

					EGameLighting	();

	// switches on, loading what is missing; false + weather_error when the weather cannot be read
	bool			Enable			();
	void			Disable			();
	// re-reads the weather file and the game's settings
	bool			Reload			();
	bool			SetWeather		(LPCSTR name, float at_hour);

	// the weathers the linked game has, file names without .ltx
	void			ListWeathers	(xr_vector<xr_string>& out) const;
	// Dead Air's Atmosfear weather for the first night of a new game at `at_hour`
	xr_string		DefaultWeather	(float at_hour) const;

	float			Exposure		() const;
	// the game's tonemap scale for a frame of mean luminance `lum` (r2_rendertarget_phase_luminance, DA's
	// bloom_luminance_3): middlegray / (lum * amount + lowlum) with the settings blended by amount, in [0.7, 20]
	float			TonemapScale	(float lum) const;
	// the background under the scene: the weather's fog colour, exposed
	u32				ClearColor		() const;

	void			Bind			();
	void			Unbind			();
	// measures a finished scene pass in `target` (w x h used) the way the game's tonemap does; true when the
	// adapted exposure moved enough that the frame is worth drawing again
	bool			Measure			(ID3D11Texture2D* target, u32 w, u32 h);
	void			Destroy			();

private:
	bool			LoadWeather		();
	void			LoadGameSettings();
	bool			Upload			();

	float						m_DrawnExposure	= 0.f;		// what the last upload drew with
	ID3D11Texture2D*			m_Staging	= nullptr;		// Measure's readback copy
	ID3D11Texture2D*			m_Data		= nullptr;
	ID3D11ShaderResourceView*	m_DataView	= nullptr;
	ID3D11Texture2D*			m_Roof		= nullptr;
	ID3D11ShaderResourceView*	m_RoofView	= nullptr;
	bool						m_Bound		= false;
};

extern ECORE_API EGameLighting GGameLighting;
