use super::*;
use gitcomet_core::domain::{Commit, CommitId};
use gitcomet_core::history_perf::{self, Work};
use std::rc::Rc;

fn commit(id: &str, parent_ids: &[&str]) -> Commit {
    Commit {
        id: CommitId(id.into()),
        parent_ids: parent_ids.iter().map(|p| CommitId((*p).into())).collect(),
        summary: "".into(),
        author: "".into(),
        time: std::time::SystemTime::UNIX_EPOCH,
    }
}

type GraphPaint = Rc<dyn Fn(&mut Window, &mut App)>;

struct GraphPaintView(GraphPaint);

impl gpui::Render for GraphPaintView {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let paint = self.0.clone();
        gpui::canvas(|_, _, _| (), move |_, (), window, cx| paint(window, cx)).size_full()
    }
}

/// A merge hands the lane that reached it from beside the node on to one of
/// its parents, so that lane joins the node and then carries on down its own
/// column. Its top half is an elbow into the node, so nothing is on its
/// column at the centre line; the straight run the continuation pass used to
/// draw from there started out of thin air, a flat-topped stub beside the
/// join's elbow (#713). The lane has to leave the node through an elbow too.
#[gpui::test]
fn a_lane_a_merge_hands_on_leaves_the_node_instead_of_a_stub(cx: &mut gpui::TestAppContext) {
    struct Shape {
        name: &'static str,
        commits: Vec<Commit>,
        merge_row: usize,
        /// Columns whose lane joins the merge and carries on below it.
        handed_on: &'static [usize],
        /// Columns whose lane runs straight past the merge.
        passing: &'static [usize],
        /// The join elbows plus one exit elbow per distinct column.
        elbows: u64,
    }
    let shapes = [
        // The issue's shape: `side` reaches the merge from the next column
        // and carries on to `feature`, the merge's second parent, so the
        // merge edge and the handed-on lane share one elbow.
        Shape {
            name: "handed on to the merge's own edge",
            commits: vec![
                commit("tip", &["merge"]),
                commit("side", &["merge"]),
                commit("merge", &["main", "feature"]),
                commit("feature", &["root"]),
                commit("main", &["root"]),
                commit("root", &[]),
            ],
            merge_row: 2,
            handed_on: &[1],
            passing: &[],
            elbows: 2,
        },
        // A lane nearer the node already runs to `feature`, so the merge
        // edge goes there and the handed-on lane has no edge of its own: it
        // needs its own elbow out, or it hangs below the node unconnected.
        Shape {
            name: "handed on beside another lane to the same parent",
            commits: vec![
                commit("tip", &["merge"]),
                commit("other", &["feature"]),
                commit("side", &["merge"]),
                commit("merge", &["main", "feature"]),
                commit("feature", &["root"]),
                commit("main", &["root"]),
                commit("root", &[]),
            ],
            merge_row: 3,
            handed_on: &[2],
            passing: &[1],
            elbows: 3,
        },
    ];

    let (view, cx) = cx.add_window_view(|_, _| GraphPaintView(Rc::new(|_, _| {})));
    for shape in shapes {
        let name = shape.name;
        let theme = AppTheme::gitcomet_dark();
        let graph =
            history_graph::compute_graph(&shape.commits, theme, std::iter::empty::<&str>(), None);
        let row = graph[shape.merge_row].clone();
        assert!(row.is_merge, "{name}: fixture row is the merge");
        assert_eq!(row.node_col, 0, "{name}: fixture merge sits on column 0");
        for &col in shape.handed_on {
            assert!(
                row.joins_in
                    .iter()
                    .any(|edge| usize::from(edge.from_col) == col),
                "{name}: column {col} must join the merge"
            );
            let lane = row.lanes_next[col];
            assert!(
                lane.is_active() && !lane.starts_at_node(),
                "{name}: and carry on below it as the same lane"
            );
            assert!(
                graph[shape.merge_row + 1].lanes_now[col].incoming(),
                "{name}: the row below continues it from the top"
            );
        }

        let (lanes, xs, elbows) = paint_row(&view, cx, theme, &row, |_| px(200.0));
        let on_col = |col: usize| straight_runs_at(&lanes, xs[col]);
        for &col in shape.handed_on {
            assert_eq!(
                on_col(col),
                0,
                "{name}: column {col} must not run straight down from the centre: {lanes:?}"
            );
        }
        for &col in [0].iter().chain(shape.passing) {
            assert_eq!(
                on_col(col),
                2,
                "{name}: column {col} runs the full height: {lanes:?}"
            );
        }
        assert_eq!(
            elbows, shape.elbows,
            "{name}: one elbow into the node per join, one out per column"
        );
    }
}

/// Too narrow to show every column, the graph draws the handed-on lane and a
/// lane running straight past the merge on one edge line. The handed-on lane
/// still leaves the node through an elbow, one stroke with the merge edge to
/// the same x, and leaving it out of the continuations must not take the edge
/// line away from the lane that runs straight.
#[gpui::test]
fn a_handed_on_lane_on_the_edge_line_leaves_the_straight_lane_beside_it(
    cx: &mut gpui::TestAppContext,
) {
    let commits = [
        commit("tip", &["merge"]),
        commit("other", &["feature"]),
        commit("side", &["merge"]),
        commit("merge", &["main", "feature"]),
        commit("feature", &["root"]),
        commit("main", &["root"]),
        commit("root", &[]),
    ];
    let theme = AppTheme::gitcomet_dark();
    let graph = history_graph::compute_graph(&commits, theme, std::iter::empty::<&str>(), None);
    let row = graph[3].clone();
    assert!(
        row.is_merge && row.node_col == 0,
        "fixture row is the merge"
    );
    assert!(
        row.joins_in.iter().any(|edge| edge.from_col == 2)
            && row.lanes_next[2].is_active()
            && !row.lanes_next[2].starts_at_node(),
        "column 2 joins the merge and carries on below it"
    );

    let (view, cx) = cx.add_window_view(|_, _| GraphPaintView(Rc::new(|_, _| {})));
    // Columns from 1 on all land on the edge line.
    let (lanes, xs, elbows) = paint_row(&view, cx, theme, &row, |grid| {
        grid.margin_x + grid.col_gap + grid.margin_right
    });
    assert!(
        xs[0] != xs[1] && xs[1] == xs[2],
        "columns 1 and 2 share the edge line: {xs:?}"
    );
    assert_eq!(
        straight_runs_at(&lanes, xs[0]),
        2,
        "the node's lane runs the full height: {lanes:?}"
    );
    assert_eq!(
        straight_runs_at(&lanes, xs[1]),
        2,
        "the lane running past the merge keeps the edge line below the centre: {lanes:?}"
    );
    assert_eq!(
        elbows, 2,
        "one elbow in from the edge line, one out to it for the merge edge and the handed-on lane"
    );
}

/// Paints `row` into a cell `width(grid)` wide. Returns the straight lane runs
/// (the unrounded quads), each column's x and the number of elbows (paths).
fn paint_row(
    view: &gpui::Entity<GraphPaintView>,
    cx: &mut gpui::VisualTestContext,
    theme: AppTheme,
    row: &history_graph::GraphRow,
    width: impl FnOnce(GraphGrid) -> Pixels,
) -> (Vec<gpui::Quad>, Vec<f32>, u64) {
    let _capture = history_perf::capture();
    let (quads, xs) = cx.update(|window, app| {
        window.set_scale_factor(1.0);
        let scale = ui_scale::UiScale::from_window(window).with_appearance(theme.metrics);
        let grid = GraphGrid::new(window, scale);
        let cell = Bounds::new(
            point(px(0.0), px(0.0)),
            size(
                width(grid),
                super::super::history::history_row_height(scale),
            ),
        );
        let col_left =
            |col: usize| grid.snap(cell.left()) + grid.col_x(col, grid.edge_x(cell.size.width));
        let xs: Vec<f32> = (0..row.lanes_now.len().max(row.lanes_next.len()))
            .map(|col| f32::from(col_left(col)))
            .collect();
        let painted_row = row.clone();
        view.update(app, |view, cx| {
            view.0 = Rc::new(move |window, app| {
                let row = &painted_row;
                paint_history_graph(
                    theme,
                    row,
                    0,
                    None,
                    true,
                    false,
                    None,
                    history_canvas::HistoryRowVisual::new(
                        theme,
                        row,
                        0,
                        None,
                        false,
                        theme.colors.surface.canvas,
                    ),
                    &HistoryAuthorVm::new("Ada Lovelace".into()),
                    1.0,
                    0.0,
                    cell,
                    grid,
                    graph_node_bounds(row, false, 1.0, cell, grid, scale),
                    window,
                    app,
                );
            });
            cx.notify();
        });
        let _ = window.draw(app);
        (window.painted_quads(), xs)
    });
    let lanes = quads
        .into_iter()
        .filter(|quad| quad.corner_radii.top_left.0 == 0.0)
        .collect();
    (lanes, xs, history_perf::count(Work::PaintPath))
}

fn straight_runs_at(lanes: &[gpui::Quad], x: f32) -> usize {
    lanes
        .iter()
        .filter(|quad| (quad.bounds.center().x.0 - x).abs() < 0.5)
        .count()
}
