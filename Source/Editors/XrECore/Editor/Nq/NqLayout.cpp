#include "stdafx.h"
#include "NqLayout.h"
#include "NqCatalog.h"
#include "NqUtil.h"

namespace
{
	// what a line shows of a value: text as the author wrote it, a reference by what it
	// names, anything else as its short summary
	xr_string LineValue(const NqCatalog::SParam& p, const SNqValue& v)
	{
		if (p.type == "text")	return NqText::Preview(v);
		if (p.type == "lua")	return "Lua";
		if (v.IsString())		return v.s;
		if (v.IsBool())			return v.b ? "yes" : "no";
		if (v.IsNumber())		return v.AsString();
		return v.IsTable() ? NqText::ValueSummary(v, 64) : xr_string();
	}

	xr_string CondTitle(LPCSTR kind)
	{
		const NqCatalog::SKind* k = NqCatalog::Find(kind);
		return k && !k->title.empty() ? k->title : xr_string(kind ? kind : "");
	}

	// What a condition checks, after its title: the values it is set to in catalog order.
	// A flag that is on reads as its name, a nested list of conditions as their titles.
	xr_string CondValue(const SNqCond& c)
	{
		const NqCatalog::SKind* kind = NqCatalog::Find(c.kind.c_str());
		if (!kind)
		{
			const xr_string sum = NqText::ValueSummary(c.params, 64);
			return sum == "{}" ? xr_string() : sum;
		}
		xr_string out;
		for (u32 i = 0; i < kind->params.size(); ++i)
		{
			const NqCatalog::SParam& p = kind->params[i];
			const SNqValue* v = c.params.Get(p.name.c_str());
			if (!v || v->IsNil() || v->IsEmpty()) continue;
			xr_string part;
			if (p.type == "cond_list" && v->IsTable())
			{
				for (u32 j = 0; j < v->arr.size(); ++j)
				{
					if (!part.empty()) part += " or ";
					part += CondTitle(v->arr[j].GetString("kind").c_str());
				}
			}
			else if (v->IsBool())	part = v->b ? p.name : xr_string();
			else					part = LineValue(p, *v);
			if (part.empty()) continue;
			if (!out.empty()) out += " ";
			out += part;
		}
		return out;
	}
}

xr_string NqLayout::CondKey(LPCSTR title, bool negate)
{
	xr_string key = title ? title : "";
	if (!negate) return key;
	// "Not has money", but "Not NPC alive": an acronym keeps its capitals
	if (key.size() > 1 && key[0] >= 'A' && key[0] <= 'Z' && !(key[1] >= 'A' && key[1] <= 'Z')) key[0] = char(key[0] + 0x20);
	return "Not " + key;
}

void NqLayout::Pins(const SNqNode& n, xr_vector<xr_string>& pins, int* declared)
{
	pins.clear();
	if (const NqCatalog::SKind* kind = NqCatalog::Find(n.kind.c_str()))
	{
		if (kind->pins_from_cases)
			if (const SNqValue* cases = n.params.Get("cases"))
				if (cases->IsTable())
					for (u32 i = 0; i < cases->arr.size(); ++i)
					{
						xr_string name = cases->arr[i].GetString("name");
						if (!name.empty()) pins.push_back(name);
					}
		for (u32 i = 0; i < kind->pins.size(); ++i)
			if (kind->pins[i] != "cases") pins.push_back(kind->pins[i]);
	}
	// everything pushed so far is what the kind declares; what the file adds below
	// would be an E007 pin, so automatic wiring must not aim at it
	if (declared) *declared = (int)pins.size();
	// pins present in the file but unknown to the catalog still get a handle
	for (u32 i = 0; i < n.out.size(); ++i)
		if (std::find(pins.begin(), pins.end(), n.out[i].first) == pins.end()) pins.push_back(n.out[i].first);
}

void NqLayout::Lines(const SNqNode& n, xr_vector<SLine>& out)
{
	out.clear();
	// the conditions first: for a wait or a trigger they are what the node is about
	for (u32 i = 0; i < n.cond.size(); ++i)
	{
		const SNqCond& c = n.cond[i];
		SLine line;
		line.cond = c.kind;
		line.negate = c.negate;
		line.key = CondKey(CondTitle(c.kind.c_str()).c_str(), c.negate);
		line.value = CondValue(c);
		out.push_back(line);
	}
	if (const NqCatalog::SKind* kind = NqCatalog::Find(n.kind.c_str()))
		for (u32 i = 0; i < kind->params.size(); ++i)
		{
			const NqCatalog::SParam& p = kind->params[i];
			// the case names are the pins already
			if (p.type == "cases_cond" || p.type == "cases_weight") continue;
			const SNqValue* v = n.params.Get(p.name.c_str());
			if (!v || v->IsNil() || v->IsEmpty()) continue;
			SLine line;
			line.key = p.name;
			line.value = LineValue(p, *v);
			if (!line.value.empty()) out.push_back(line);
		}
	// what does not fit is counted on the last line: the Details panel has all of it
	if ((int)out.size() > kMaxLines)
	{
		const int hidden = (int)out.size() - (kMaxLines - 1);
		out.resize(kMaxLines - 1);
		SLine more;
		more.value = NqUtil::Format("+%d more", hidden);
		out.push_back(more);
	}
}

void NqLayout::Box(const SNqNode& n, SBox& out)
{
	out = SBox();
	xr_vector<SLine> lines;
	Lines(n, lines);
	xr_vector<xr_string> pins;
	Pins(n, pins);

	float y = kHeader;
	out.lines = (int)lines.size();
	if (out.lines)
	{
		y += kPad;
		out.lines_y = y;
		y += kLine * float(out.lines);
	}
	// a trigger has no actions to run: the runtime reads on_enter / on_exit of a main node only
	out.actions = NqText::IsTrigger(n.kind.c_str()) ? 0 : int(n.on_enter.size() + n.on_exit.size());
	out.action_rows = _min(out.actions, kMaxActions);
	out.more = out.actions > kMaxActions;
	if (out.actions)
	{
		y += kPad;
		out.actions_y = y;
		y += kActionRow * float(out.action_rows + (out.more ? 1 : 0));
	}
	// a single "next" pin needs no name
	out.pin_labels = pins.size() > 1 || (pins.size() == 1 && pins[0] != "next");
	if (out.pin_labels)
	{
		y += kPad * 0.5f;
		out.pins_y = y;
		y += kPinRow;
	}
	else y += kPad;
	out.h = y;
}

Fvector2 NqLayout::NodeSize(const SNqNode& n)
{
	SBox box;
	Box(n, box);
	Fvector2 s;
	s.set(box.w, box.h);
	return s;
}

namespace
{
	struct SEdge
	{
		int from, to;
		float side, order;
		bool active;
	};

	struct SLayoutNode
	{
		int layer;
		int parent;
		int file_order;
		float side_sum;
		int side_count;
		float order_key;
		float x, y, w, h;
		float center_x;
		float offset_x;
		float min_x, max_x;
		xr_vector<int> children;
	};

	struct SBlock
	{
		float min_x, max_x;
		SBlock() : min_x(0.f), max_x(0.f) {}
	};

	struct SVisit
	{
		int node;
		u32 next_edge;
	};

	float Snap(float v) { return floorf(v / NqLayout::kGrid + 0.5f) * NqLayout::kGrid; }

	float PinSide(int index, int count)
	{
		if (count <= 1) return 0.f;
		return 2.f * float(index + 1) / float(count + 1) - 1.f;
	}

	SBlock PackBlock(xr_vector<SLayoutNode>& nodes, const xr_vector<int>& members, float gap)
	{
		SBlock block;
		float cursor = 0.f;
		for (u32 i = 0; i < members.size(); ++i)
		{
			SLayoutNode& child = nodes[members[i]];
			child.offset_x = cursor - child.min_x;
			cursor += child.max_x - child.min_x;
			if (i + 1 < members.size()) cursor += gap;
		}
		block.max_x = cursor;
		return block;
	}

	void ShiftBlock(xr_vector<SLayoutNode>& nodes, const xr_vector<int>& members, SBlock& block, float shift)
	{
		for (u32 i = 0; i < members.size(); ++i) nodes[members[i]].offset_x += shift;
		block.min_x += shift;
		block.max_x += shift;
	}

	void PackRoots(xr_vector<SLayoutNode>& nodes, const xr_vector<int>& roots)
	{
		if (roots.empty()) return;
		float cursor = 0.f;
		for (u32 i = 0; i < roots.size(); ++i)
		{
			SLayoutNode& root = nodes[roots[i]];
			root.center_x = cursor - root.min_x;
			cursor += root.max_x - root.min_x;
			if (i + 1 < roots.size()) cursor += NqLayout::kTreeGapX;
		}
		const float shift = -cursor * 0.5f;
		for (u32 i = 0; i < roots.size(); ++i) nodes[roots[i]].center_x += shift;
	}

	int BranchUnder(int ancestor, int node, const xr_vector<SLayoutNode>& nodes)
	{
		if (node == ancestor) return -1;
		int current = node;
		while (nodes[current].parent >= 0 && nodes[current].parent != ancestor)
			current = nodes[current].parent;
		return nodes[current].parent == ancestor ? current : -1;
	}
}

float NqLayout::Sane(float v)
{
	if (!_finite(v) || _isnan(v)) return 0.f;
	if (v < -kWorldMax) return -kWorldMax;
	if (v > kWorldMax) return kWorldMax;
	return v;
}

int NqLayout::SanePositions(SNqQuest& q)
{
	int fixed = 0;
	for (u32 i = 0; i < q.nodes.size(); ++i)
	{
		Fvector2& p = q.nodes[i].pos;
		const float x = Sane(p.x), y = Sane(p.y);
		// NaN compares unequal to itself, so this also catches it.
		if (!(x == p.x) || !(y == p.y)) { p.set(x, y); ++fixed; }
	}
	return fixed;
}

int NqLayout::EnsurePositions(SNqQuest& q)
{
	SanePositions(q);
	for (const SNqNode& node : q.nodes)
		if (!node.has_pos) return Run(q, true);
	return 0;
}

int NqLayout::Run(SNqQuest& q, bool only_missing)
{
	const u32 count = (u32)q.nodes.size();
	if (!count) return 0;
	SanePositions(q);

	const int virtual_root = (int)count;
	xr_vector<SLayoutNode> layout(count + 1);
	for (u32 i = 0; i <= count; ++i)
	{
		SLayoutNode& node = layout[i];
		node.layer = 0;
		node.parent = -1;
		node.file_order = (int)i;
		node.side_sum = 0.f;
		node.side_count = 0;
		node.order_key = (float)i;
		node.x = node.y = node.center_x = node.offset_x = 0.f;
		node.w = node.h = 0.f;
		node.min_x = node.max_x = 0.f;
		if (i < count)
		{
			const Fvector2 size = NodeSize(q.nodes[i]);
			node.w = size.x;
			node.h = size.y;
			node.min_x = -size.x * 0.5f;
			node.max_x = size.x * 0.5f;
		}
	}

	xr_vector<SEdge> edges;
	xr_vector<xr_vector<int> > outgoing(count);
	xr_vector<xr_string> pins;
	for (u32 source_index = 0; source_index < count; ++source_index)
	{
		const SNqNode& source = q.nodes[source_index];
		Pins(source, pins);
		for (u32 visual_pin = 0; visual_pin < pins.size(); ++visual_pin)
		{
			const SNqPin* output = 0;
			for (u32 p = 0; p < source.out.size(); ++p)
				if (source.out[p].first == pins[visual_pin]) { output = &source.out[p]; break; }
			if (!output) continue;
			for (u32 target_order = 0; target_order < output->second.size(); ++target_order)
			{
				const int target = q.NodeIndex(output->second[target_order].c_str());
				if (target < 0 || NqText::IsTrigger(q.nodes[target].kind.c_str())) continue;
				SEdge edge;
				edge.from = (int)source_index;
				edge.to = target;
				edge.side = PinSide((int)visual_pin, (int)pins.size());
				edge.order = (float)visual_pin + float(target_order + 1) / float(output->second.size() + 1);
				edge.active = false;
				outgoing[source_index].push_back((int)edges.size());
				edges.push_back(edge);
			}
		}
	}

	// Only DFS back-edges are removed; all other shared paths remain structural.
	xr_vector<u8> color(count, 0);
	xr_vector<int> finish;
	xr_vector<SVisit> stack;
	auto visit = [&](int start)
	{
		stack.clear();
		SVisit first;
		first.node = start;
		first.next_edge = 0;
		stack.push_back(first);
		color[start] = 1;
		while (!stack.empty())
		{
			SVisit& frame = stack.back();
			if (frame.next_edge >= outgoing[frame.node].size())
			{
				color[frame.node] = 2;
				finish.push_back(frame.node);
				stack.pop_back();
				continue;
			}
			SEdge& edge = edges[outgoing[frame.node][frame.next_edge++]];
			if (color[edge.to] == 1) continue;
			edge.active = true;
			if (color[edge.to] != 0) continue;
			color[edge.to] = 1;
			SVisit child;
			child.node = edge.to;
			child.next_edge = 0;
			stack.push_back(child);
		}
	};
	for (u32 i = 0; i < count; ++i)
		if (NqText::IsTrigger(q.nodes[i].kind.c_str()) && color[i] == 0) visit((int)i);
	for (u32 i = 0; i < count; ++i)
		if (color[i] == 0) visit((int)i);

	xr_vector<int> topo;
	for (int i = (int)finish.size() - 1; i >= 0; --i) topo.push_back(finish[i]);
	xr_vector<xr_vector<int> > incoming(count);
	xr_vector<xr_vector<int> > unique_parents(count);
	for (u32 edge_index = 0; edge_index < edges.size(); ++edge_index)
	{
		const SEdge& edge = edges[edge_index];
		if (!edge.active) continue;
		incoming[edge.to].push_back((int)edge_index);
		bool found = false;
		for (u32 p = 0; p < unique_parents[edge.to].size(); ++p)
			if (unique_parents[edge.to][p] == edge.from) { found = true; break; }
		if (!found) unique_parents[edge.to].push_back(edge.from);
	}

	int max_layer = 0;
	for (u32 oi = 0; oi < topo.size(); ++oi)
	{
		const int source = topo[oi];
		for (u32 e = 0; e < outgoing[source].size(); ++e)
		{
			const SEdge& edge = edges[outgoing[source][e]];
			if (!edge.active) continue;
			layout[edge.to].layer = std::max(layout[edge.to].layer, layout[source].layer + 1);
			max_layer = std::max(max_layer, layout[edge.to].layer);
		}
	}

	// A DAG dominator tree owns exclusive branches and promotes every join into one merge block.
	xr_vector<xr_vector<u8> > dominators(count + 1, xr_vector<u8>(count + 1, 0));
	xr_vector<int> dominator_count(count + 1, 0);
	dominators[virtual_root][virtual_root] = 1;
	dominator_count[virtual_root] = 1;
	for (u32 oi = 0; oi < topo.size(); ++oi)
	{
		const int node_index = topo[oi];
		xr_vector<u8>& current = dominators[node_index];
		if (unique_parents[node_index].empty())
			current = dominators[virtual_root];
		else
		{
			current = dominators[unique_parents[node_index][0]];
			for (u32 p = 1; p < unique_parents[node_index].size(); ++p)
			{
				const xr_vector<u8>& other = dominators[unique_parents[node_index][p]];
				for (u32 d = 0; d <= count; ++d) current[d] = current[d] && other[d];
			}
		}
		current[node_index] = 1;
		int best = virtual_root;
		int best_depth = dominator_count[virtual_root];
		int depth = 0;
		for (u32 d = 0; d <= count; ++d)
		{
			if (current[d]) ++depth;
			if ((int)d != node_index && current[d] && dominator_count[d] > best_depth)
			{
				best = (int)d;
				best_depth = dominator_count[d];
			}
		}
		dominator_count[node_index] = depth;
		layout[node_index].parent = best;
	}

	xr_vector<float> order_min(count, 1.0e30f);
	xr_vector<float> order_max(count, -1.0e30f);
	for (u32 edge_index = 0; edge_index < edges.size(); ++edge_index)
	{
		const SEdge& edge = edges[edge_index];
		if (!edge.active || unique_parents[edge.to].size() != 1 || layout[edge.to].parent != edge.from) continue;
		SLayoutNode& child = layout[edge.to];
		child.side_sum += edge.side;
		++child.side_count;
		order_min[edge.to] = std::min(order_min[edge.to], edge.order);
		order_max[edge.to] = std::max(order_max[edge.to], edge.order);
	}
	for (u32 i = 0; i < count; ++i)
		if (unique_parents[i].size() == 1 && order_min[i] <= order_max[i])
			layout[i].order_key = (order_min[i] + order_max[i]) * 0.5f;

	float root_order = 0.f;
	for (u32 i = 0; i < count; ++i)
		if (NqText::IsTrigger(q.nodes[i].kind.c_str()) && layout[i].parent == virtual_root)
			layout[i].order_key = root_order++;
	for (u32 i = 0; i < count; ++i)
		if (!NqText::IsTrigger(q.nodes[i].kind.c_str()) && unique_parents[i].empty() && layout[i].parent == virtual_root)
			layout[i].order_key = root_order++;

	for (u32 oi = 0; oi < topo.size(); ++oi)
	{
		const int node_index = topo[oi];
		if (unique_parents[node_index].size() <= 1) continue;
		const int ancestor = layout[node_index].parent;
		xr_vector<int> branches;
		float min_key = 1.0e30f, max_key = -1.0e30f;
		float min_side = 1.0e30f, max_side = -1.0e30f;
		for (u32 p = 0; p < unique_parents[node_index].size(); ++p)
		{
			const int parent = unique_parents[node_index][p];
			const int branch = BranchUnder(ancestor, parent, layout);
			if (branch >= 0)
			{
				bool found = false;
				for (u32 b = 0; b < branches.size(); ++b)
					if (branches[b] == branch) { found = true; break; }
				if (found) continue;
				branches.push_back(branch);
				const SLayoutNode& anchor = layout[branch];
				const float side = anchor.side_count ? anchor.side_sum / float(anchor.side_count) : 0.f;
				min_key = std::min(min_key, anchor.order_key);
				max_key = std::max(max_key, anchor.order_key);
				min_side = std::min(min_side, side);
				max_side = std::max(max_side, side);
				continue;
			}
			for (u32 e = 0; e < incoming[node_index].size(); ++e)
			{
				const SEdge& edge = edges[incoming[node_index][e]];
				if (edge.from != ancestor) continue;
				min_key = std::min(min_key, edge.order);
				max_key = std::max(max_key, edge.order);
				min_side = std::min(min_side, edge.side);
				max_side = std::max(max_side, edge.side);
			}
		}
		if (min_key <= max_key) layout[node_index].order_key = (min_key + max_key) * 0.5f;
		if (min_side <= max_side)
		{
			layout[node_index].side_sum = (min_side + max_side) * 0.5f;
			layout[node_index].side_count = 1;
		}
	}

	for (u32 i = 0; i < count; ++i) layout[layout[i].parent].children.push_back((int)i);
	for (u32 i = 0; i <= count; ++i)
	{
		xr_vector<int>& children = layout[i].children;
		std::stable_sort(children.begin(), children.end(), [&](int a, int b)
		{
			if (layout[a].order_key != layout[b].order_key) return layout[a].order_key < layout[b].order_key;
			if (layout[a].layer != layout[b].layer) return layout[a].layer < layout[b].layer;
			return layout[a].file_order < layout[b].file_order;
		});
	}

	// Children are processed before dominators, so every packed block uses complete descendant bounds.
	for (int oi = (int)topo.size() - 1; oi >= 0; --oi)
	{
		const int index = topo[oi];
		SLayoutNode& node = layout[index];
		xr_vector<int> left, middle, right;
		for (u32 c = 0; c < node.children.size(); ++c)
		{
			const int child_index = node.children[c];
			const SLayoutNode& child = layout[child_index];
			const float side = child.side_count ? child.side_sum / float(child.side_count) : 0.f;
			if (side < -0.001f) left.push_back(child_index);
			else if (side > 0.001f) right.push_back(child_index);
			else middle.push_back(child_index);
		}

		SBlock middle_block;
		if (middle.size() == 1)
		{
			SLayoutNode& child = layout[middle[0]];
			child.offset_x = 0.f;
			middle_block.min_x = child.min_x;
			middle_block.max_x = child.max_x;
		}
		else if (!middle.empty())
		{
			middle_block = PackBlock(layout, middle, kGapX);
			ShiftBlock(layout, middle, middle_block, -(middle_block.min_x + middle_block.max_x) * 0.5f);
		}

		SBlock left_block;
		if (!left.empty())
		{
			left_block = PackBlock(layout, left, kGapX);
			const float edge = middle.empty() ? -kGapX * 0.5f : middle_block.min_x - kGapX;
			ShiftBlock(layout, left, left_block, edge - left_block.max_x);
		}
		SBlock right_block;
		if (!right.empty())
		{
			right_block = PackBlock(layout, right, kGapX);
			const float edge = middle.empty() ? kGapX * 0.5f : middle_block.max_x + kGapX;
			ShiftBlock(layout, right, right_block, edge - right_block.min_x);
		}

		node.min_x = -node.w * 0.5f;
		node.max_x = node.w * 0.5f;
		if (!left.empty()) { node.min_x = std::min(node.min_x, left_block.min_x); node.max_x = std::max(node.max_x, left_block.max_x); }
		if (!middle.empty()) { node.min_x = std::min(node.min_x, middle_block.min_x); node.max_x = std::max(node.max_x, middle_block.max_x); }
		if (!right.empty()) { node.min_x = std::min(node.min_x, right_block.min_x); node.max_x = std::max(node.max_x, right_block.max_x); }
	}

	PackRoots(layout, layout[virtual_root].children);
	for (u32 oi = 0; oi < topo.size(); ++oi)
	{
		SLayoutNode& node = layout[topo[oi]];
		if (node.parent != virtual_root) node.center_x = layout[node.parent].center_x + node.offset_x;
	}

	xr_vector<float> row_height(max_layer + 1, 0.f);
	xr_vector<float> row_y(max_layer + 1, 0.f);
	for (u32 i = 0; i < count; ++i)
		row_height[layout[i].layer] = std::max(row_height[layout[i].layer], layout[i].h);
	float y = 0.f;
	for (int layer = 0; layer <= max_layer; ++layer)
	{
		row_y[layer] = Snap(y);
		y = row_y[layer] + row_height[layer] + kGapY;
	}
	for (u32 i = 0; i < count; ++i)
	{
		layout[i].x = Sane(Snap(layout[i].center_x - layout[i].w * 0.5f));
		layout[i].y = Sane(row_y[layout[i].layer]);
	}

	int moved = 0;
	if (only_missing)
	{
		bool any_existing = false;
		float max_y = -1e9f, min_missing_y = 1e9f;
		for (u32 i = 0; i < count; ++i)
		{
			if (q.nodes[i].has_pos) { any_existing = true; max_y = std::max(max_y, q.nodes[i].pos.y + layout[i].h); }
			else min_missing_y = std::min(min_missing_y, layout[i].y);
		}
		const float dy = any_existing ? Snap(max_y + kGapY - min_missing_y) : 0.f;
		for (u32 i = 0; i < count; ++i)
		{
			if (q.nodes[i].has_pos) continue;
			q.nodes[i].pos.set(layout[i].x, Sane(layout[i].y + dy));
			q.nodes[i].has_pos = true;
			++moved;
		}
		return moved;
	}
	for (u32 i = 0; i < count; ++i)
	{
		if (!q.nodes[i].has_pos || q.nodes[i].pos.x != layout[i].x || q.nodes[i].pos.y != layout[i].y) ++moved;
		q.nodes[i].pos.set(layout[i].x, layout[i].y);
		q.nodes[i].has_pos = true;
	}
	return moved;
}
