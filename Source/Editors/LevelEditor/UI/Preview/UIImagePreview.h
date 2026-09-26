#pragma once

// Texture viewer: the double-click destination for image assets, the editor's
// "texture" tab. Modelled on Unreal's Texture Editor - the picture on a canvas
// that pans and zooms at the mouse, and beside it the facts you need about it:
// format, size, mips (switchable), alpha, sRGB, what it costs on the GPU and on
// disk, and where it came from.
//
// It owns the resources it shows, so closing the tab (or opening another
// texture) frees them - nothing stays resident behind the user's back.

class UIImagePreview : public XrUI
{
public:
					UIImagePreview		();
	virtual			~UIImagePreview		();

	// Opens/focuses the window on `name`. `source` is the content browser's:
	// 0 project (a path under the project root), 1 the shared SDK library (an
	// FS-resolvable texture name), 2 the linked game install (may live inside an
	// archive). Returns false and fills `err` when the image cannot be decoded.
	static bool		Show				(LPCSTR name, int source, xr_string* err);
	static void		Close				();
	static IC bool	IsOpen				() { return !!Form; }
	// frees what a closed or switched texture left for the frame that still drew it
	static void		Update				();
	// MCP texture_preview: what the tab shows, as JSON fields (no braces)
	static void		McpState			(xr_string& out);
	// The view asked for over MCP: zoom 0 fits, below 0 keeps; mip below 0 keeps; channel 0 RGBA, 1 R, 2 G,
	// 3 B, 4 A, below 0 keeps; details 1 shows the Details pane, 0 hides it, below 0 keeps.
	static bool		McpView				(float zoom, int mip, int channel, int details, xr_string& err);
	// the tab was drawn in the last frame: what McpState says it laid out is what shows
	static bool		DrawnLately			();
	virtual void	Draw				();

private:
	static UIImagePreview*	Form;
	// the texture preview is the editor's "texture" tab (XUI::Shell)
	static void		ShowTab				();

	// the decoded texture and the view of the mip currently on screen. Both are
	// only ever swapped OUTSIDE this window's own draw (double-click runs in the
	// browser's draw, which precedes this window; MCP runs between frames; the
	// mip menu changes before DrawImage records anything) - so the old view is
	// never sitting in an unrendered ImGui draw list when it is released.
	ID3D11Texture2D*		m_Tex;
	ID3D11ShaderResourceView* m_View;
	D3D11_TEXTURE2D_DESC	m_Desc;

	xr_string		m_Name;
	xr_string		m_File;			// the file on disk it came from; empty for one inside the game's archives
	xr_string		m_Archive;		// gamedata-relative path of a texture the game keeps in its archives
	// where the file lives, apart from its path: the Details' Source row and the MCP `origin`
	enum EOrigin { oNone, oProject, oLibrary, oGameFolder, oGameArchive };
	EOrigin			m_Origin;
	xr_string		m_Container;	// DDS, TGA, PNG ...
	bool			m_Decoded;		// the GPU could not take the file as it is: mip 0 decoded to B8G8R8A8
	u32				m_FileSize;
	xr_string		m_Error;

	u32				m_Mip;			// which mip the view shows
	float			m_Zoom;			// 0 = fit to the canvas, else screen pixels per texel
	ImVec2			m_Pan;			// px: where a zoomed image sits off its centred place
	int				m_Channels;		// 0 RGBA, 1 R, 2 G, 3 B, 4 A
	bool			m_ShowDetails;
	float			m_Scale;		// what the last frame drew at, fit included
	float			m_DetailsW;		// pt, as the last frame laid it out
	int				m_DrawnFrame;	// the ImGui frame that drew the tab last

	void			Release				();
	bool			Adopt				(ID3D11ShaderResourceView* srv);
	bool			AdoptTexture		(ID3D11Texture2D* tex);
	bool			ViewMip				(u32 mip);
	void			DrawToolBar			(XUI::Toolbar& tb);
	void			DrawDetails			();
	void			DrawImage			(const ImVec2& p0, const ImVec2& avail);
	void			DrawStatus			();
	xr_string		FormatText			() const;
	// size of the mip currently displayed
	IC u32			MipW				() const { u32 w = m_Desc.Width  >> m_Mip; return w ? w : 1; }
	IC u32			MipH				() const { u32 h = m_Desc.Height >> m_Mip; return h ? h : 1; }
};
