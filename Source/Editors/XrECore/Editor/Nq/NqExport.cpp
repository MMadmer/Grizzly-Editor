#include "stdafx.h"
#include "NqExport.h"
#include "NqDoc.h"
#include "NqLua.h"
#include "NqUtil.h"
#include "NqValidate.h"
#include "../EditorProject.h"

bool NqExport::ValidateProject(LPCSTR project_root, xr_string& err, xr_vector<xr_string>& log,
							   int* errors_out, int* warnings_out)
{
	err.clear();
	log.clear();
	if (errors_out) *errors_out = 0;
	if (warnings_out) *warnings_out = 0;
	if (!project_root || !project_root[0]) return true;

	// what ships is what is on disk: open documents with unsaved edits are
	// noted, not validated in their place
	xr_vector<NqDocs::SProjectQuest> quests;
	NqDocs::InvalidateProjectIndex();
	NqDocs::ProjectQuests(quests, true);
	if (quests.empty()) return true;

	int errors = 0, warnings = 0;
	for (u32 i = 0; i < quests.size(); ++i)
	{
		const xr_string rel = NqUtil::ProjectRelative(quests[i].path.c_str());
		if (NqDoc* d = NqDocs::Get(quests[i].path.c_str()))
			if (d->dirty) log.push_back(NqUtil::Format("%s: ~ has unsaved changes in the editor - the file on disk is what ships", rel.c_str()));
		SNqQuest q;
		xr_string rerr;
		if (!NqDocs::ReadQuest(quests[i].path.c_str(), q, rerr, true))
		{
			++errors;
			log.push_back(NqUtil::Format("%s: E003 %s", rel.c_str(), rerr.c_str()));
			continue;
		}
		xr_vector<SNqProblem> problems;
		NqDocs::Validate(q, quests[i].path.c_str(), problems, true);
		for (u32 p = 0; p < problems.size(); ++p)
		{
			if (problems[p].IsError()) ++errors; else ++warnings;
			log.push_back(rel + ": " + problems[p].Text());
		}
	}
	// E113: a behaviour graph that runs itself through a chain of others. One
	// document cannot see it, so it is looked for here, where all of them are.
	{
		xr_vector<xr_string>				ids, rels;
		xr_vector< xr_vector<xr_string> >	calls;
		for (u32 i = 0; i < quests.size(); ++i)
		{
			if (!quests[i].behaviour || !quests[i].readable) continue;
			SNqQuest q;
			xr_string rerr;
			if (!NqDocs::ReadQuest(quests[i].path.c_str(), q, rerr, true)) continue;
			xr_vector<xr_string> out_calls;
			for (u32 n = 0; n < q.nodes.size(); ++n)
				if (q.nodes[n].kind == "behaviour.run")
				{
					const xr_string asset = q.nodes[n].params.GetString("asset", "");
					if (!asset.empty() && asset.find('.') == xr_string::npos) out_calls.push_back(asset);
				}
			ids.push_back(q.id);
			rels.push_back(NqUtil::ProjectRelative(quests[i].path.c_str()));
			calls.push_back(out_calls);
		}
		for (u32 s = 0; s < ids.size(); ++s)
		{
			// can `s` be reached from what it calls? (the direct case is the
			// single-document check's; a chain of two or more lands here)
			xr_vector<u32> stack;
			xr_vector<bool> seen; seen.assign(ids.size(), false);
			for (u32 c = 0; c < calls[s].size(); ++c)
				for (u32 k = 0; k < ids.size(); ++k)
					if (ids[k] == calls[s][c] && k != s && !seen[k]) { seen[k] = true; stack.push_back(k); }
			bool loops = false;
			while (!stack.empty() && !loops)
			{
				const u32 at = stack.back(); stack.pop_back();
				for (u32 c = 0; c < calls[at].size() && !loops; ++c)
				{
					if (calls[at][c] == ids[s]) { loops = true; break; }
					for (u32 k = 0; k < ids.size(); ++k)
						if (ids[k] == calls[at][c] && !seen[k]) { seen[k] = true; stack.push_back(k); }
				}
			}
			if (loops)
			{
				++errors;
				log.push_back(NqUtil::Format("%s: E113 behaviour graph '%s' runs itself through the graphs it calls", rels[s].c_str(), ids[s].c_str()));
			}
		}
	}
	for (u32 i = 0; i < log.size(); ++i) Msg("%s [nq] %s", (log[i].find(": E") != xr_string::npos) ? "!" : "~", log[i].c_str());
	if (errors_out) *errors_out = errors;
	if (warnings_out) *warnings_out = warnings;
	if (errors)
	{
		err = NqUtil::Format("quest graph errors: %d (see log)", errors);
		return false;
	}
	return true;
}

xr_string NqExport::TemplateText(LPCSTR id)
{
	SNqQuest q;
	q.nq = 1;
	q.id = (id && id[0]) ? id : "new_quest";
	q.title = SNqValue::String("New quest");
	q.activation = "auto";
	SNqNode start;
	start.id = "start"; start.kind = "trigger.start";
	start.Connect("next", "finish");
	SNqNode finish;
	finish.id = "finish"; finish.kind = "flow.end";
	finish.params = SNqValue::Table();
	finish.params.Set("status", SNqValue::String("completed"));
	q.nodes.push_back(start);
	q.nodes.push_back(finish);
	return NqLua::Write(q);
}

xr_string NqExport::BehaviourTemplateText(LPCSTR id)
{
	SNqQuest q;
	q.nq	= 1;
	q.type	= "behaviour";
	q.id	= (id && id[0]) ? id : "new_behaviour";
	q.title	= SNqValue::String("New behaviour");
	// one role to start from: whoever runs this binds it, and every step
	// says who = "squad" to address it
	SNqRole role;
	role.name	= "squad";
	role.title	= SNqValue::String("Squad");
	role.many	= true;
	q.roles.push_back(role);

	SNqNode start;
	start.id = "start"; start.kind = "trigger.start";
	start.Connect("next", "finish");
	SNqNode finish;
	finish.id = "finish"; finish.kind = "flow.end";
	finish.params = SNqValue::Table();
	finish.params.Set("status", SNqValue::String("completed"));
	q.nodes.push_back(start);
	q.nodes.push_back(finish);
	return NqLua::Write(q);
}

bool NqExport::NewBehaviourTemplate(LPCSTR path, xr_string& err)
{
	err.clear();
	if (!path || !path[0]) { err = "path is empty"; return false; }
	if (!NqUtil::IsBehaviourPath(path)) { err = "a behaviour graph is a .behasset"; return false; }
	if (NqUtil::FileExists(path)) { err = "file already exists"; return false; }
	xr_string id = NqText::IdFromName(NqUtil::BaseName(path).c_str());
	if (id.empty()) id = "new_behaviour";
	if (!NqLua::WriteFile(path, BehaviourTemplateText(id.c_str()), err)) return false;
	NqDocs::InvalidateProjectIndex();
	return true;
}

bool NqExport::NewQuestTemplate(LPCSTR path, xr_string& err)
{
	err.clear();
	if (!path || !path[0]) { err = "path is empty"; return false; }
	if (NqUtil::FileExists(path)) { err = "file already exists"; return false; }
	// the file name is the id, by the rule W061 checks against
	xr_string id = NqText::IdFromName(NqUtil::BaseName(path).c_str());
	if (id.empty()) id = "new_quest";
	if (!NqLua::WriteFile(path, TemplateText(id.c_str()), err)) return false;
	NqDocs::InvalidateProjectIndex();
	return true;
}
