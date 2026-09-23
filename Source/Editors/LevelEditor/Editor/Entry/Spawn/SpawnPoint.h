#pragma once

#define SPAWNPOINT_VERSION   			0x0017

// refs
class ISE_Visual;
class ISE_Motion;
class CObjectAnimator;

class CLE_Visual :private pureDrawUI
{
    virtual void OnDrawUI();
public:
    static bool     g_tmp_lock;
    ISE_Visual* source;
    IRenderVisual* visual;
    void 			OnChangeVisual();
    void			StopAllAnimations();
    void 			PlayAnimation();
    void 			PlayAnimationFirstFrame();
    void 			PlayAnimationLastFrame();
    void 			PauseAnimation();
public:
    CLE_Visual(ISE_Visual* src);
    virtual			~CLE_Visual();
};
// A placed stalker's standing order: what it does from the moment it spawns until a
// quest tells it otherwise. The export writes it into the NPC's custom data as
// [xms_order]; the game's orders runtime adopts it once (game repo:
// docs/dead-air/NQ_RUNTIME.md, "Standing orders").
struct SXmsOrder
{
	enum EKind : u8 { okNone = 0, okGuard, okPatrol, okWander, okFollow, okCount };
	enum EFlags : u8
	{
		flRun		= 1 << 0,	// patrol / wander: run instead of walk
		flHostile	= 1 << 1,	// an enemy of the player from the start
	};
	u8				kind		= okNone;
	shared_str		route;				// patrol: the way path walked
	shared_str		look;				// guard: a way path watched from the post
	float			radius		= 20.f;	// guard: the post; wander: the area around the spawn
	shared_str		target;				// follow: a story id; empty = the player
	float			distance	= 3.f;	// follow: how close it keeps
	Flags8			flags;

					SXmsOrder	() { flags.zero(); }
	bool			Empty		() const { return kind == okNone && !flags.test(flHostile); }
	static LPCSTR	KindKey		(u8 kind);		// "guard", ... ; null for none
	static u8		KindFromKey	(LPCSTR key);	// okNone when unknown
	// the [xms_order] section, CRLF lines; empty when there is no order
	void			Section		(xr_string& out) const;
};

class CSpawnPoint : public CCustomObject,
	public CPhysicsShellHolderEditorBase
{
	typedef CCustomObject inherited;

    friend class    SceneBuilder;
public:

    class CLE_Motion
    {
    public:
        ISE_Motion*		source;
        CObjectAnimator*animator;
        void 			OnChangeMotion	();
        void 			PlayMotion		();
    public:
						CLE_Motion		(ISE_Motion* src);
        virtual			~CLE_Motion		();
    };
	struct SSpawnData: public ISE_AbstractLEOwner
	{
		CLASS_ID		m_ClassID;
        shared_str 		m_Profile;
		ISE_Abstract*	m_Data;
        CLE_Visual*		m_Visual;
        Flags8			m_flags;
        xr_vector<CLE_Visual*> m_VisualHelpers;

        CLE_Motion*		m_Motion;
        CSpawnPoint*	m_owner;
        enum{ eSDTypeRespawn = (1<<0),};
        SSpawnData	(CSpawnPoint* o)
        {
			m_ClassID	= 0;
			m_Data		= 0;
            m_Visual	= 0;
            m_Motion	= 0;
            m_owner		= o;
            m_flags.zero();
        }
        ~SSpawnData	()
        {
        	Destroy	();
        }
        void			Create			(LPCSTR entity_ref);
        void			Destroy			();
        bool			Valid			()const{return m_Data;}

        bool		    LoadStream		(IReader&);
        void		    SaveStream		(IWriter&);
        bool		    LoadLTX			(CInifile& ini, LPCSTR sect_name);
        void		    SaveLTX			(CInifile& ini, LPCSTR sect_name);

		bool 			ExportGame		(SExportStreams* F, CSpawnPoint* owner);
        void            ExportSpawn     (xr_vector<NET_Packet>& Ps, CSpawnPoint* owner);
        void            PreExportSpawn( CSpawnPoint* owner);

		void			FillProp		(LPCSTR pref, PropItemVec& values);

		void    		Render			(bool bSelected, const Fmatrix& parent,int priority, bool strictB2F);
		void    		OnFrame			();
    	void 	OnAnimControlClick		(ButtonValue* value, bool& bModif, bool& bSafe);  
        
		virtual void get_bone_xform				(LPCSTR name, Fmatrix& xform);
        IC ISE_Abstract* GetEntity()const { return m_Data; }
	};

	SSpawnData    	m_SpawnData;
	CCustomObject*	m_AttachedObject;
	// only a stalker takes one (CanTakeOrder)
	SXmsOrder		m_Order;

    EPointType		m_Type;
    shared_str		m_rpProfile;
    union{
    	struct{
		    u8					m_RP_TeamID;
		    u8					m_RP_Type;
			GameTypeChooser     m_GameType;
        };
        struct{
        	float	m_EM_Radius;
            float 	m_EM_Power;
            float	m_EM_ViewDist;
            u32		m_EM_FogColor;
            float	m_EM_FogDensity;
            u32		m_EM_AmbientColor;
            u32		m_EM_SkyColor;
            u32		m_EM_HemiColor;
            Flags16	m_EM_Flags;
        };
    };

    shared_str		SectionToEditor			(shared_str);
    shared_str		EditorToSection			(shared_str);
	void 	OnProfileChange			(PropValue* prop);
	void 	OnRPointTypeChange		(PropValue* prop);
    
	void 	OnEnvModFlagChange		(PropValue* prop);
	void 	OnFillChooseItems		(ChooseValue*);
    void 	OnFillRespawnItemProfile(ChooseValue*);
	void	OnFillOrderPaths		(ChooseValue*);
	void	OnFillOrderTargets		(ChooseValue*);
	void	OnOrderKindChange		(PropValue* prop);
	void	FillOrderProps			(LPCSTR pref, PropItemVec& values);
    bool 			OnAppendObject			(CCustomObject* object);
protected:
    virtual void 	SetPosition		(const Fvector& pos);
    virtual void 	SetRotation		(const Fvector& rot);
    virtual void 	SetScale		(const Fvector& scale);
protected:
	virtual void 	Move			(Fvector& amount);
public:
	                CSpawnPoint    	(LPVOID data, LPCSTR name);
    void            Construct   	(LPVOID data);
    virtual         ~CSpawnPoint   	();
    virtual bool	CanAttach		() {return true;}
    
	bool 			RefCompare		(LPCSTR ref);
    virtual LPCSTR	RefName			();
	// a shape entity with no shape is invalid; give it a default one
	void			EnsureShape		();

    bool			CreateSpawnData	(LPCSTR entity_ref);
	virtual void    Render      	( int priority, bool strictB2F );

	virtual bool    RayPick     	( float& distance,	const Fvector& start,	const Fvector& direction, SRayPickInfo* pinf = NULL );
    virtual bool 	FrustumPick		( const CFrustum& frustum );
	virtual bool    GetBox      	(Fbox& box) ;

	virtual void 	OnFrame			();

	virtual void 	Select			(int  flag);

	virtual bool 	LoadStream		(IReader&);
	virtual bool 	LoadLTX			(CInifile& ini, LPCSTR sect_name);
	virtual void 	SaveStream		 (IWriter&);
	virtual void 	SaveLTX			(CInifile& ini, LPCSTR sect_name);

    virtual bool	ExportGame		(SExportStreams* data);

	virtual void	FillProp		(LPCSTR pref, PropItemVec& values);

    bool			AttachObject	(CCustomObject* obj);
    void			DetachObject	();
    
    virtual bool	OnChooseQuery	(LPCSTR specific);
    virtual	void	OnSceneRemove	();
    		void 	UseSimulatePose ();
    virtual	void  ExportSpawn(xr_vector<NET_Packet>& Ps);
    virtual void	PreExportSpawn();
	bool IsGraphPoint()const;
	// a stalker: the orders runtime drives stalker schemes only
	bool			CanTakeOrder	() const;
public:

    IC ISE_Abstract* GetEntity() { PreExportSpawn(); return m_SpawnData.GetEntity(); }
    virtual	IKinematics*	   _BCL	ObjectKinematics	 ()				 	{ if(!m_SpawnData.m_Visual||!m_SpawnData.m_Visual->visual)return 0;return m_SpawnData.m_Visual->visual->dcast_PKinematics();}
private:
    virtual void 	OnUpdateTransform();
private:
    void			RenderSimBox	();
};
