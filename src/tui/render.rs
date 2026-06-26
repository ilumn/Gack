use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::prelude::Frame;
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, Paragraph, StatefulWidget, Widget, Wrap,
};

use crate::app::App;
use crate::commands;
use crate::git::{ChangeKind, ChangeSection, DiffKind, DiffTarget};
use crate::model::{Focus, Modal, Panel};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(frame, vertical[0], app);

    if vertical[1].width < 92 {
        draw_narrow(frame, vertical[1], app);
    } else {
        let panels = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(40), Constraint::Min(40)])
            .split(vertical[1]);
        draw_rail(frame, panels[0], app);
        draw_inspector(frame, panels[1], app);
    }

    draw_status(frame, vertical[2], app);
    draw_modal(frame, area, app);
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let (conflicts, staged, unstaged, untracked) = app.section_counts();
    let repo = &app.state.repo;
    let ahead = repo.ahead.map(|n| format!(" +{n}")).unwrap_or_default();
    let behind = repo.behind.map(|n| format!(" -{n}")).unwrap_or_default();
    let tabs = [
        Panel::Changes,
        Panel::Stash,
        Panel::Branches,
        Panel::Log,
        Panel::Remotes,
        Panel::Repos,
    ]
    .into_iter()
    .map(|panel| {
        if panel == app.state.panel {
            format!("[{}]", panel.label())
        } else {
            panel.label().to_string()
        }
    })
    .collect::<Vec<_>>()
    .join(" ");
    let operation = if app.state.rebase_state.is_some() {
        "  REBASE"
    } else {
        ""
    };
    let title = format!(
        " {}  {}{}{}{}  S:{} U:{} ?:{} C:{}   {} ",
        repo.root_label,
        repo.branch,
        ahead,
        behind,
        operation,
        staged,
        unstaged,
        untracked,
        conflicts,
        tabs
    );
    Paragraph::new(title)
        .style(
            Style::default()
                .bg(ui_color(Color::Rgb(31, 35, 40)))
                .fg(ui_color(Color::White)),
        )
        .render(area, frame.buffer_mut());
}

fn draw_narrow(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match app.state.focus {
        Focus::Changes => draw_rail(frame, area, app),
        Focus::Diff => draw_inspector(frame, area, app),
    }
}

fn draw_rail(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match app.state.panel {
        Panel::Changes => draw_changes(frame, area, app),
        Panel::Stash => draw_stash(frame, area, app),
        Panel::Branches => draw_branches(frame, area, app),
        Panel::Log => draw_log(frame, area, app),
        Panel::Remotes => draw_remotes(frame, area, app),
        Panel::Repos => draw_repos(frame, area, app),
    }
}

fn draw_inspector(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match app.state.panel {
        Panel::Changes => draw_diff(frame, area, app),
        Panel::Stash => draw_stash_inspector(frame, area, app),
        Panel::Branches => draw_branch_inspector(frame, area, app),
        Panel::Log => draw_log_inspector(frame, area, app),
        Panel::Remotes => draw_remote_inspector(frame, area, app),
        Panel::Repos => draw_repo_inspector(frame, area, app),
    }
}

fn draw_changes(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.state.focus == Focus::Changes && app.state.modal == Modal::None;
    let title = if focused { "Changes active" } else { "Changes" };
    let block = panel_block(title, focused);
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());

    if app.state.changes.is_empty() {
        let empty = vec![
            Line::from("Working tree is clean").bold(),
            Line::from(""),
            Line::from("No staged, unstaged, or untracked changes."),
            Line::from("Press r to refresh or q to quit.").dim(),
        ];
        Paragraph::new(empty)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .render(inner, frame.buffer_mut());
        return;
    }

    let items = change_items(app, inner.width);
    let mut state =
        ratatui::widgets::ListState::default().with_selected(selected_visual_index(app));
    let highlight_symbol = if focused { "> " } else { "| " };
    let highlight_style = if focused {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
            .fg(ui_color(Color::Rgb(121, 192, 255)))
            .add_modifier(Modifier::BOLD)
    };
    StatefulWidget::render(
        List::new(items)
            .highlight_symbol(highlight_symbol)
            .highlight_style(highlight_style),
        inner,
        frame.buffer_mut(),
        &mut state,
    );
}

fn change_items(app: &App, width: u16) -> Vec<ListItem<'static>> {
    let mut items = Vec::new();
    let mut last_section: Option<ChangeSection> = None;

    for change in &app.state.changes {
        if last_section != Some(change.section) {
            last_section = Some(change.section);
            items.push(ListItem::new(Line::from(vec![Span::styled(
                format!(" {} ", change.section.label()),
                Style::default()
                    .fg(ui_color(Color::Rgb(139, 148, 158)))
                    .add_modifier(Modifier::BOLD),
            )])));
        }

        let marked = if app.state.marked.contains(&change.key) {
            "[x]"
        } else {
            "[ ]"
        };
        let path_width = (width as usize).saturating_sub(14).max(8);
        let path = truncate_middle(&change.display_path(), path_width);
        let line = Line::from(vec![
            Span::raw(format!("{marked} ")),
            Span::styled(
                format!("{:<2}", change.kind.tag()),
                Style::default()
                    .fg(kind_color(change.kind))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                path,
                Style::default().fg(ui_color(Color::Rgb(225, 228, 232))),
            ),
            Span::styled(
                format!(" {}", change.xy),
                Style::default().fg(ui_color(Color::Rgb(110, 118, 129))),
            ),
        ]);
        items.push(ListItem::new(line));
    }

    items
}

fn selected_visual_index(app: &App) -> Option<usize> {
    let selected = app.state.selected_change()?;
    let mut visual_index = 0;
    let mut last_section: Option<ChangeSection> = None;

    for change in &app.state.changes {
        if last_section != Some(change.section) {
            visual_index += 1;
            last_section = Some(change.section);
        }
        if change.id == selected.id {
            return Some(visual_index);
        }
        visual_index += 1;
    }

    None
}

fn draw_stash(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.state.focus == Focus::Changes && app.state.modal == Modal::None;
    let block = panel_block(active_title("Stash", focused), focused);
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());

    if app.state.stashes.is_empty() {
        draw_empty(
            frame,
            inner,
            "No stashes",
            "No saved stash entries. Press r to refresh.",
        );
        return;
    }

    let items = app
        .state
        .stashes
        .iter()
        .map(|stash| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    truncate_end(&stash.selector, 10),
                    Style::default()
                        .fg(ui_color(Color::Rgb(121, 192, 255)))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    truncate_middle(
                        &stash.subject,
                        (inner.width as usize).saturating_sub(22).max(8),
                    ),
                    Style::default().fg(ui_color(Color::Rgb(225, 228, 232))),
                ),
                Span::styled(
                    format!(" {}", truncate_end(&stash.relative_time, 9)),
                    Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
                ),
            ]))
        })
        .collect::<Vec<_>>();
    render_list(frame, inner, items, Some(app.state.selected_stash), focused);
}

fn draw_branches(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.state.focus == Focus::Changes && app.state.modal == Modal::None;
    let block = panel_block(active_title("Branches", focused), focused);
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());

    if app.state.branches.is_empty() {
        draw_empty(
            frame,
            inner,
            "No branches",
            "This repository has no branch refs yet.",
        );
        return;
    }

    let mut items = Vec::new();
    let mut selected_visual = None;
    let mut last_remote: Option<bool> = None;
    for (index, branch) in app.state.branches.iter().enumerate() {
        if last_remote != Some(branch.remote) {
            last_remote = Some(branch.remote);
            items.push(section_item(if branch.remote { "REMOTE" } else { "LOCAL" }));
        }
        if index == app.state.selected_branch {
            selected_visual = Some(items.len());
        }
        let marker = if branch.current { "*" } else { " " };
        let upstream = branch.upstream.as_deref().unwrap_or("no upstream");
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                format!("{marker} "),
                Style::default()
                    .fg(ui_color(Color::Rgb(63, 185, 80)))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                truncate_middle(
                    &branch.name,
                    (inner.width as usize).saturating_sub(18).max(8),
                ),
                Style::default().fg(ui_color(Color::Rgb(225, 228, 232))),
            ),
            Span::styled(
                format!(" {}", truncate_end(upstream, 12)),
                Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
            ),
        ])));
    }
    render_list(frame, inner, items, selected_visual, focused);
}

fn draw_log(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.state.focus == Focus::Changes && app.state.modal == Modal::None;
    let block = panel_block(active_title("Log", focused), focused);
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());

    if app.state.log.is_empty() {
        draw_empty(
            frame,
            inner,
            "No commits",
            "History appears empty for this repository.",
        );
        return;
    }

    let items = app
        .state
        .log
        .iter()
        .map(|entry| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{} ", entry.short_oid),
                    Style::default()
                        .fg(ui_color(Color::Rgb(121, 192, 255)))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    truncate_middle(
                        &entry.subject,
                        (inner.width as usize).saturating_sub(10).max(8),
                    ),
                    Style::default().fg(ui_color(Color::Rgb(225, 228, 232))),
                ),
            ]))
        })
        .collect::<Vec<_>>();
    render_list(frame, inner, items, Some(app.state.selected_log), focused);
}

fn draw_remotes(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.state.focus == Focus::Changes && app.state.modal == Modal::None;
    let block = panel_block(active_title("Remotes", focused), focused);
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());

    if app.state.remotes.is_empty() {
        draw_empty(
            frame,
            inner,
            "No remotes",
            "Add a remote with Git, then refresh.",
        );
        return;
    }

    let items = app
        .state
        .remotes
        .iter()
        .map(|remote| {
            let url = remote
                .fetch_url
                .as_deref()
                .or(remote.push_url.as_deref())
                .unwrap_or("no url");
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<10}", truncate_end(&remote.name, 10)),
                    Style::default()
                        .fg(ui_color(Color::Rgb(121, 192, 255)))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    truncate_middle(url, (inner.width as usize).saturating_sub(12).max(8)),
                    Style::default().fg(ui_color(Color::Rgb(225, 228, 232))),
                ),
            ]))
        })
        .collect::<Vec<_>>();
    render_list(
        frame,
        inner,
        items,
        Some(app.state.selected_remote),
        focused,
    );
}

fn draw_repos(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.state.focus == Focus::Changes && app.state.modal == Modal::None;
    let block = panel_block(active_title("Repos", focused), focused);
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());

    if app.state.worktrees.is_empty() && app.state.submodules.is_empty() {
        draw_empty(
            frame,
            inner,
            "No repo topology",
            "No worktrees or submodules found.",
        );
        return;
    }

    let mut items = Vec::new();
    let mut selected_visual = None;
    if !app.state.worktrees.is_empty() {
        items.push(section_item("WORKTREES"));
        for (index, worktree) in app.state.worktrees.iter().enumerate() {
            if app.state.selected_repo == index {
                selected_visual = Some(items.len());
            }
            let marker = if worktree.current { "*" } else { " " };
            let branch = worktree.branch.as_deref().unwrap_or(if worktree.detached {
                "detached"
            } else {
                "unknown"
            });
            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{marker} "),
                    Style::default()
                        .fg(ui_color(Color::Rgb(63, 185, 80)))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    truncate_middle(
                        &path_label(&worktree.path),
                        (inner.width as usize).saturating_sub(16).max(8),
                    ),
                    Style::default().fg(ui_color(Color::Rgb(225, 228, 232))),
                ),
                Span::styled(
                    format!(" {}", truncate_end(branch, 12)),
                    Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
                ),
            ])));
        }
    }
    if !app.state.submodules.is_empty() {
        items.push(section_item("SUBMODULES"));
        for (index, submodule) in app.state.submodules.iter().enumerate() {
            let repo_index = app.state.worktrees.len() + index;
            if app.state.selected_repo == repo_index {
                selected_visual = Some(items.len());
            }
            let state = if !submodule.initialized {
                "uninit"
            } else if submodule.dirty {
                "dirty"
            } else {
                "clean"
            };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    "S ",
                    Style::default()
                        .fg(ui_color(Color::Rgb(210, 168, 255)))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    truncate_middle(
                        &submodule.path.to_string_lossy(),
                        (inner.width as usize).saturating_sub(10).max(8),
                    ),
                    Style::default().fg(ui_color(Color::Rgb(225, 228, 232))),
                ),
                Span::styled(
                    format!(" {state}"),
                    Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
                ),
            ])));
        }
    }
    render_list(frame, inner, items, selected_visual, focused);
}

fn draw_diff(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.state.focus == Focus::Diff && app.state.modal == Modal::None;
    if app
        .state
        .selected_change()
        .is_some_and(|change| change.is_conflict())
    {
        draw_conflict_inspector(frame, area, app, focused);
        return;
    }
    let title = app
        .state
        .selected_change()
        .map(|change| {
            let active = if focused { " active" } else { "" };
            format!(
                "Diff{active} {}",
                truncate_middle(&change.display_path(), 48)
            )
        })
        .unwrap_or_else(|| {
            if focused {
                "Diff active".to_string()
            } else {
                "Diff".to_string()
            }
        });
    draw_diff_snapshot(
        frame,
        area,
        &title,
        "Select a change to view its diff.",
        app.state.diff.as_ref(),
        app,
        focused,
    );
}

fn draw_conflict_inspector(frame: &mut Frame<'_>, area: Rect, app: &App, focused: bool) {
    let title = app
        .state
        .selected_change()
        .map(|change| format!("Conflict {}", truncate_middle(&change.display_path(), 42)))
        .unwrap_or_else(|| "Conflict".to_string());
    let block = panel_block(title, focused);
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());

    let Some(detail) = &app.state.conflict_detail else {
        draw_empty(
            frame,
            inner,
            "Conflict details unavailable",
            "Press r to refresh conflict state.",
        );
        return;
    };

    let mut lines = vec![
        Line::from(Span::styled(
            detail.path.to_string_lossy(),
            Style::default()
                .fg(ui_color(Color::Rgb(225, 228, 232)))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Stages",
            Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
        )),
    ];
    for stage in &detail.stages {
        let label = match stage.stage {
            1 => "base",
            2 => "ours",
            3 => "theirs",
            _ => "stage",
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{label:<8}"),
                Style::default().fg(ui_color(Color::Rgb(121, 192, 255))),
            ),
            Span::styled(
                truncate_end(&stage.oid, 12),
                Style::default().fg(ui_color(Color::Rgb(225, 228, 232))),
            ),
            Span::styled(
                format!(" {}", stage.mode),
                Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from("1 ours | 2 theirs | A mark resolved | e editor"));

    Paragraph::new(lines)
        .scroll((scroll_offset(app.state.inspector_scroll), 0))
        .wrap(Wrap { trim: false })
        .render(inner, frame.buffer_mut());
}

fn draw_stash_inspector(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.state.focus == Focus::Diff && app.state.modal == Modal::None;
    let title = app
        .state
        .selected_stash()
        .map(|stash| format!("Stash patch {}", stash.selector))
        .unwrap_or_else(|| "Stash patch".to_string());
    draw_diff_snapshot(
        frame,
        area,
        &title,
        "Select a stash to view its patch.",
        app.state.stash_patch.as_ref(),
        app,
        focused,
    );
}

fn draw_log_inspector(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.state.focus == Focus::Diff && app.state.modal == Modal::None;
    let title = app
        .state
        .selected_log_entry()
        .map(|entry| format!("Commit {}", entry.short_oid))
        .unwrap_or_else(|| "Commit".to_string());
    draw_diff_snapshot(
        frame,
        area,
        &title,
        "Select a commit to view its patch.",
        app.state.log_patch.as_ref(),
        app,
        focused,
    );
}

fn draw_diff_snapshot(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    empty_message: &str,
    diff: Option<&crate::git::Diff>,
    app: &App,
    focused: bool,
) {
    let block = panel_block(title, focused);
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());

    let Some(diff) = diff else {
        Paragraph::new(empty_message)
            .alignment(Alignment::Center)
            .render(inner, frame.buffer_mut());
        return;
    };

    let mut lines = Vec::new();
    let target = match diff.target {
        DiffTarget::Staged => "staged",
        DiffTarget::Worktree => "unstaged",
    };
    let binary = if diff.is_binary { " binary" } else { "" };
    let hunk_label = if diff.hunks.is_empty() {
        "no hunks".to_string()
    } else {
        format!(
            "hunk {}/{}",
            app.state.diff_hunk.saturating_add(1),
            diff.hunks.len()
        )
    };
    let hunk_header = diff
        .hunks
        .get(app.state.diff_hunk)
        .map(|hunk| truncate_end(&hunk.header, inner.width.saturating_sub(42) as usize))
        .unwrap_or_default();
    lines.push(Line::from(vec![
        Span::styled(
            format!("{target}{binary}  "),
            Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
        ),
        Span::styled(
            format!("{hunk_label}  "),
            Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
        ),
        Span::styled(
            format!("+{} -{}  ", diff.additions, diff.deletions),
            Style::default()
                .fg(ui_color(Color::Rgb(139, 148, 158)))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "ctx:{} ws:{}  ",
                diff.context_lines,
                diff.whitespace.label()
            ),
            Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
        ),
        Span::styled(
            hunk_header,
            Style::default().fg(ui_color(Color::Rgb(121, 192, 255))),
        ),
    ]));

    let skip = app.state.diff_scroll;
    let height = inner.height.saturating_sub(1) as usize;
    for (idx, diff_line) in diff.lines.iter().enumerate().skip(skip).take(height) {
        let mut style = match diff_line.kind {
            DiffKind::Add => Style::default().fg(ui_color(Color::Rgb(63, 185, 80))),
            DiffKind::Delete => Style::default().fg(ui_color(Color::Rgb(248, 81, 73))),
            DiffKind::Hunk => Style::default()
                .fg(ui_color(Color::Rgb(121, 192, 255)))
                .add_modifier(Modifier::BOLD),
            DiffKind::Header => Style::default()
                .fg(ui_color(Color::Rgb(210, 168, 255)))
                .add_modifier(Modifier::BOLD),
            DiffKind::Metadata => Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
            DiffKind::Context => Style::default().fg(ui_color(Color::Rgb(201, 209, 217))),
        };

        let selected_hunk_line = diff
            .hunks
            .get(app.state.diff_hunk)
            .is_some_and(|hunk| hunk.start_line == idx);

        if selected_hunk_line && focused {
            style = style.add_modifier(Modifier::REVERSED);
        } else if selected_hunk_line {
            style = style.add_modifier(Modifier::BOLD);
        }

        let marker = if selected_hunk_line { "> " } else { "  " };
        let line_width = inner.width.saturating_sub(2) as usize;
        lines.push(Line::from(vec![
            Span::styled(
                marker,
                Style::default().fg(ui_color(Color::Rgb(121, 192, 255))),
            ),
            Span::styled(truncate_end(&diff_line.text, line_width), style),
        ]));
    }

    Paragraph::new(lines).render(inner, frame.buffer_mut());
}

fn draw_branch_inspector(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.state.focus == Focus::Diff && app.state.modal == Modal::None;
    let block = panel_block(active_title("Branch", focused), focused);
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());

    let Some(branch) = app.state.selected_branch() else {
        draw_empty(
            frame,
            inner,
            "No branch selected",
            "Select a branch to inspect it.",
        );
        return;
    };

    let refs = if branch.current { "HEAD" } else { "none" };
    let lines = vec![
        Line::from(Span::styled(
            &branch.name,
            Style::default()
                .fg(ui_color(Color::Rgb(225, 228, 232)))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        detail_line(
            "kind",
            if branch.remote { "remote" } else { "local" },
            inner.width,
        ),
        detail_line(
            "current",
            if branch.current { "yes" } else { "no" },
            inner.width,
        ),
        detail_line("head", &branch.short_oid, inner.width),
        detail_line(
            "upstream",
            branch.upstream.as_deref().unwrap_or("none"),
            inner.width,
        ),
        detail_line("full ref", &branch.full_ref, inner.width),
        detail_line("refs", refs, inner.width),
        Line::from(""),
        Line::from(Span::styled(
            truncate_end(&branch.subject, inner.width as usize),
            Style::default().fg(ui_color(Color::Rgb(201, 209, 217))),
        )),
    ];
    Paragraph::new(lines)
        .scroll((scroll_offset(app.state.inspector_scroll), 0))
        .render(inner, frame.buffer_mut());
}

fn draw_remote_inspector(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.state.focus == Focus::Diff && app.state.modal == Modal::None;
    let block = panel_block(active_title("Remote", focused), focused);
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());

    let Some(remote) = app.state.selected_remote() else {
        draw_empty(
            frame,
            inner,
            "No remote selected",
            "This repository has no remotes.",
        );
        return;
    };

    let lines = vec![
        Line::from(Span::styled(
            &remote.name,
            Style::default()
                .fg(ui_color(Color::Rgb(225, 228, 232)))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        detail_line(
            "fetch",
            remote.fetch_url.as_deref().unwrap_or("none"),
            inner.width,
        ),
        detail_line(
            "push",
            remote.push_url.as_deref().unwrap_or("none"),
            inner.width,
        ),
        Line::from(""),
        Line::from(Span::styled(
            "f fetch | U fast-forward update | P push current branch",
            Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
        )),
    ];
    Paragraph::new(lines)
        .scroll((scroll_offset(app.state.inspector_scroll), 0))
        .render(inner, frame.buffer_mut());
}

fn draw_repo_inspector(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.state.focus == Focus::Diff && app.state.modal == Modal::None;
    let block = panel_block(active_title("Repo Inspector", focused), focused);
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());

    if let Some(worktree) = app.state.selected_repo_worktree() {
        let lines = vec![
            Line::from(Span::styled(
                path_label(&worktree.path),
                Style::default()
                    .fg(ui_color(Color::Rgb(225, 228, 232)))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            detail_line("type", "worktree", inner.width),
            detail_line(
                "current",
                if worktree.current { "yes" } else { "no" },
                inner.width,
            ),
            detail_line(
                "branch",
                worktree.branch.as_deref().unwrap_or("none"),
                inner.width,
            ),
            detail_line(
                "head",
                worktree.head.as_deref().unwrap_or("unknown"),
                inner.width,
            ),
            detail_line(
                "detached",
                if worktree.detached { "yes" } else { "no" },
                inner.width,
            ),
            detail_line(
                "bare",
                if worktree.bare { "yes" } else { "no" },
                inner.width,
            ),
            detail_line(
                "locked",
                worktree.locked.as_deref().unwrap_or("no"),
                inner.width,
            ),
            detail_line(
                "prunable",
                worktree.prunable.as_deref().unwrap_or("no"),
                inner.width,
            ),
        ];
        Paragraph::new(lines)
            .scroll((scroll_offset(app.state.inspector_scroll), 0))
            .render(inner, frame.buffer_mut());
        return;
    }

    if let Some(submodule) = app.state.selected_repo_submodule() {
        let lines = vec![
            Line::from(Span::styled(
                submodule.path.to_string_lossy(),
                Style::default()
                    .fg(ui_color(Color::Rgb(225, 228, 232)))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            detail_line("type", "submodule", inner.width),
            detail_line("name", &submodule.name, inner.width),
            detail_line(
                "url",
                submodule.url.as_deref().unwrap_or("none"),
                inner.width,
            ),
            detail_line(
                "initialized",
                if submodule.initialized { "yes" } else { "no" },
                inner.width,
            ),
            detail_line(
                "dirty",
                if submodule.dirty { "yes" } else { "no" },
                inner.width,
            ),
            detail_line(
                "recorded",
                submodule.recorded_oid.as_deref().unwrap_or("unknown"),
                inner.width,
            ),
            detail_line(
                "checkout",
                submodule.checked_out_oid.as_deref().unwrap_or("unknown"),
                inner.width,
            ),
        ];
        Paragraph::new(lines)
            .scroll((scroll_offset(app.state.inspector_scroll), 0))
            .render(inner, frame.buffer_mut());
        return;
    }

    draw_empty(
        frame,
        inner,
        "No repository item selected",
        "Select a worktree or submodule.",
    );
}

fn draw_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let text = match app.state.modal {
        Modal::None => contextual_status(app, area.width as usize),
        Modal::Commit => {
            " Commit | Tab field | Enter body/newline | Ctrl+E editor | Ctrl+S commit | Esc cancel "
                .to_string()
        }
        Modal::Palette => " Command | type query | Enter run | Esc close ".to_string(),
        Modal::Confirm => " Confirm operation | Enter confirm | Esc cancel ".to_string(),
        Modal::Help => " Esc/q close ".to_string(),
        Modal::Error if app.state.retry_handoff.is_some() => {
            " Error | e retry in terminal | Esc/q close ".to_string()
        }
        Modal::Error => " Esc/q close ".to_string(),
    };
    Paragraph::new(truncate_end(&text, area.width as usize))
        .style(
            Style::default()
                .bg(ui_color(Color::Rgb(22, 27, 34)))
                .fg(ui_color(Color::Rgb(201, 209, 217))),
        )
        .render(area, frame.buffer_mut());
}

fn contextual_status(app: &App, width: usize) -> String {
    let mark_text = match app.state.marked.len() {
        0 => "no marks".to_string(),
        1 => "1 marked".to_string(),
        n => format!("{n} marked"),
    };

    let panel_help = if app
        .state
        .selected_change()
        .is_some_and(|change| change.is_conflict())
    {
        "Conflict | 1 ours | 2 theirs | A mark resolved | r refresh"
    } else if app.state.rebase_state.is_some() {
        "Rebase | C continue | K skip | Q abort | Enter inspect conflicts"
    } else {
        match (app.state.panel, app.state.focus) {
            (Panel::Changes, Focus::Changes) => {
                "Changes | j/k select | Space mark | s stage | u unstage | e edit conflict"
            }
            (Panel::Changes, Focus::Diff) => {
                "Diff | j/k scroll | n/p hunks | S stage hunk | U unstage hunk | w/+/- diff"
            }
            (Panel::Stash, Focus::Changes) => "Stash | j/k select | Enter patch | A apply | d drop",
            (Panel::Stash, Focus::Diff) => "Stash patch | j/k scroll | n/p hunks | Esc rail",
            (Panel::Branches, Focus::Changes) => {
                "Branches | j/k select | o switch local | U update ff-only"
            }
            (Panel::Branches, Focus::Diff) => {
                "Branch inspector | j/k scroll | o switch | U update | Tab rail"
            }
            (Panel::Log, Focus::Changes) => "Log | j/k select | Enter/Tab patch | L log",
            (Panel::Log, Focus::Diff) => "Commit patch | j/k scroll | n/p hunks | Esc rail",
            (Panel::Remotes, Focus::Changes) => {
                "Remotes | j/k select | f fetch | U update | P push"
            }
            (Panel::Remotes, Focus::Diff) => {
                "Remote inspector | j/k scroll | f fetch | U update | P push"
            }
            (Panel::Repos, Focus::Changes) => {
                "Repos | j/k select | x remove wt | s stage ptr | y sync | U update"
            }
            (Panel::Repos, Focus::Diff) => {
                "Repo inspector | j/k scroll | x remove wt | s stage ptr | y sync | U update"
            }
        }
    };

    let critical = " | 0 Changes | : commands | ? help | q quit ";
    let prefix_budget = width.saturating_sub(critical.chars().count()).max(12);
    format!(
        "{}{}",
        truncate_end(
            &format!(
                " {} | {} | {}",
                app.state.status_message, mark_text, panel_help
            ),
            prefix_budget,
        ),
        critical
    )
}

fn draw_modal(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match app.state.modal {
        Modal::None => {}
        Modal::Help => draw_help(frame, centered(area, 72, 22)),
        Modal::Commit => draw_commit(frame, centered(area, 76, 22), app),
        Modal::Palette => draw_palette(frame, centered(area, 72, 12), app),
        Modal::Confirm => draw_confirm(frame, centered(area, 76, 14), app),
        Modal::Error => draw_error(frame, centered(area, 76, 12), app),
    }
}

fn draw_help(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from("Panels").bold(),
        Line::from("  0 Changes  z Stash  B Branches  L Log  F Remotes  R Repos"),
        Line::from(""),
        Line::from("Navigation").bold(),
        Line::from("  j/k or arrows move   Tab switch rail/inspector   Enter inspect"),
        Line::from("  PageUp/PageDown scroll patch   n/p previous/next hunk"),
        Line::from(""),
        Line::from("Changes and diff").bold(),
        Line::from("  Space/m mark   M clear marks   s stage   u unstage   a stage all"),
        Line::from("  S stage hunk   U unstage hunk   x discard   e edit conflict"),
        Line::from("  w whitespace   +/- context"),
        Line::from(""),
        Line::from("Commit and history").bold(),
        Line::from("  c commit   Ctrl+E external editor   I interactive rebase"),
        Line::from("  A apply stash   d drop stash"),
        Line::from(""),
        Line::from("Branches, remotes, repos").bold(),
        Line::from("  o switch branch   f fetch   U ff-only update/submodule update   P push"),
        Line::from("  x remove worktree   s stage submodule pointer   y sync submodule"),
        Line::from(""),
        Line::from("Rebase and app").bold(),
        Line::from("  C continue   K skip   Q abort   e terminal retry from remote errors"),
        Line::from("  r refresh   : palette   Esc/q close"),
    ];
    Paragraph::new(lines)
        .block(modal_block("Help"))
        .wrap(Wrap { trim: false })
        .render(area, frame.buffer_mut());
}

fn draw_commit(frame: &mut Frame<'_>, area: Rect, app: &App) {
    frame.render_widget(Clear, area);
    let summary_style = if !app.state.commit_body_focus {
        Style::default()
            .fg(ui_color(Color::White))
            .add_modifier(Modifier::REVERSED)
    } else {
        Style::default().fg(ui_color(Color::White))
    };
    let body_style = if app.state.commit_body_focus {
        Style::default()
            .fg(ui_color(Color::White))
            .add_modifier(Modifier::REVERSED)
    } else {
        Style::default().fg(ui_color(Color::White))
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "Summary ",
                Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
            ),
            Span::styled(
                if app.state.commit_summary.is_empty() {
                    "Write a concise commit summary".to_string()
                } else {
                    app.state.commit_summary.clone()
                },
                summary_style,
            ),
        ]),
        commit_summary_meter(app),
        Line::from(""),
        Line::from(Span::styled(
            "Body",
            Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
        )),
    ];

    if app.state.commit_body.is_empty() {
        lines.push(Line::from(Span::styled("Optional detail", body_style)));
    } else {
        for line in app.state.commit_body.lines() {
            lines.push(Line::from(Span::styled(line.to_string(), body_style)));
        }
    }

    lines.push(Line::from(""));
    lines.extend(commit_review_lines(app, area.width));
    lines.push(Line::from(""));
    lines.push(Line::from(format!(
        "Staged files: {}   Ctrl+E editor   Ctrl+S commit   Esc cancel",
        app.state.staged_count()
    )));

    Paragraph::new(lines)
        .block(modal_block("Commit"))
        .wrap(Wrap { trim: false })
        .render(area, frame.buffer_mut());
}

fn commit_summary_meter(app: &App) -> Line<'static> {
    let summary_len = app.state.commit_summary.chars().count();
    if summary_len > 72 {
        Line::from(Span::styled(
            format!("Warning: summary is {summary_len} chars; keep it near 72"),
            Style::default().fg(ui_color(Color::Rgb(248, 81, 73))),
        ))
    } else {
        Line::from(Span::styled(
            format!("{summary_len}/72"),
            Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
        ))
    }
}

fn commit_review_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let staged = app
        .state
        .changes
        .iter()
        .filter(|change| change.is_staged())
        .take(4)
        .collect::<Vec<_>>();
    if staged.is_empty() {
        return vec![Line::from(Span::styled(
            "No staged changes",
            Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
        ))];
    }

    let mut lines = vec![Line::from(Span::styled(
        "Staged review",
        Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
    ))];
    let path_width = (width as usize).saturating_sub(18).max(8);
    for change in staged {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<2} ", change.kind.tag()),
                Style::default()
                    .fg(kind_color(change.kind))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                truncate_middle(&change.display_path(), path_width),
                Style::default().fg(ui_color(Color::Rgb(225, 228, 232))),
            ),
        ]));
    }
    let staged_count = app.state.staged_count();
    if staged_count > 4 {
        lines.push(Line::from(Span::styled(
            format!("... {} more staged", staged_count - 4),
            Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
        )));
    }
    lines
}

fn draw_palette(frame: &mut Frame<'_>, area: Rect, app: &App) {
    frame.render_widget(Clear, area);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "> ",
                Style::default().fg(ui_color(Color::Rgb(121, 192, 255))),
            ),
            Span::raw(&app.state.palette_query),
        ]),
        Line::from(""),
    ];
    let matches = commands::matching_commands(&app.state.palette_query);
    if matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "No matching commands",
            Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
        )));
    } else {
        for command in matches.into_iter().take(8) {
            let disabled = app.command_disabled_reason(command.id);
            let style = if disabled.is_some() {
                Style::default().fg(ui_color(Color::Rgb(139, 148, 158)))
            } else {
                Style::default().fg(ui_color(Color::Rgb(225, 228, 232)))
            };
            let label = if let Some(reason) = disabled {
                format!("{}  disabled: {reason}", command.label)
            } else {
                command.label.to_string()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:<4}", command.key),
                    Style::default()
                        .fg(ui_color(Color::Rgb(121, 192, 255)))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(label, style),
            ]));
        }
    }
    Paragraph::new(lines)
        .block(modal_block("Command"))
        .render(area, frame.buffer_mut());
}

fn draw_confirm(frame: &mut Frame<'_>, area: Rect, app: &App) {
    frame.render_widget(Clear, area);
    let mut lines = vec![
        Line::from(Span::styled(
            &app.state.confirm_title,
            Style::default()
                .fg(ui_color(Color::Rgb(248, 81, 73)))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    lines.extend(limited_body_lines(&app.state.confirm_body, 8));
    lines.push(Line::from(""));
    lines.push(Line::from("Enter confirm | Esc cancel"));
    Paragraph::new(lines)
        .block(modal_block("Confirm"))
        .wrap(Wrap { trim: false })
        .render(area, frame.buffer_mut());
}

fn draw_error(frame: &mut Frame<'_>, area: Rect, app: &App) {
    frame.render_widget(Clear, area);
    let mut lines = vec![
        Line::from(Span::styled(
            &app.state.error_title,
            Style::default()
                .fg(ui_color(Color::Rgb(248, 81, 73)))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    lines.extend(limited_body_lines(&app.state.error_body, 6));
    lines.push(Line::from(""));
    if app.state.retry_handoff.is_some() {
        lines.push(Line::from("e retry in terminal | Esc/q close"));
    } else {
        lines.push(Line::from(
            "Press r after closing to refresh, or Esc/q to close.",
        ));
    }
    Paragraph::new(lines)
        .block(modal_block("Error"))
        .wrap(Wrap { trim: false })
        .render(area, frame.buffer_mut());
}

fn limited_body_lines(text: &str, limit: usize) -> Vec<Line<'static>> {
    let mut lines = text
        .lines()
        .take(limit)
        .map(|line| Line::from(line.to_string()))
        .collect::<Vec<_>>();
    if text.lines().count() > limit {
        lines.push(Line::from(Span::styled(
            "... more omitted",
            Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
        )));
    }
    lines
}

fn render_list(
    frame: &mut Frame<'_>,
    area: Rect,
    items: Vec<ListItem<'static>>,
    selected: Option<usize>,
    focused: bool,
) {
    let mut state = ratatui::widgets::ListState::default().with_selected(selected);
    let highlight_symbol = if focused { "> " } else { "| " };
    let highlight_style = if focused {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
            .fg(ui_color(Color::Rgb(121, 192, 255)))
            .add_modifier(Modifier::BOLD)
    };
    StatefulWidget::render(
        List::new(items)
            .highlight_symbol(highlight_symbol)
            .highlight_style(highlight_style),
        area,
        frame.buffer_mut(),
        &mut state,
    );
}

fn draw_empty(frame: &mut Frame<'_>, area: Rect, title: &str, body: &str) {
    let lines = vec![
        Line::from(Span::styled(
            title.to_string(),
            Style::default()
                .fg(ui_color(Color::Rgb(225, 228, 232)))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            body.to_string(),
            Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
        )),
    ];
    Paragraph::new(lines)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .render(area, frame.buffer_mut());
}

fn section_item(label: &str) -> ListItem<'static> {
    ListItem::new(Line::from(vec![Span::styled(
        format!(" {} ", label),
        Style::default()
            .fg(ui_color(Color::Rgb(139, 148, 158)))
            .add_modifier(Modifier::BOLD),
    )]))
}

fn detail_line(label: &str, value: &str, width: u16) -> Line<'static> {
    let value_width = (width as usize).saturating_sub(12).max(4);
    Line::from(vec![
        Span::styled(
            format!("{label:<11}"),
            Style::default().fg(ui_color(Color::Rgb(139, 148, 158))),
        ),
        Span::styled(
            truncate_end(value, value_width),
            Style::default().fg(ui_color(Color::Rgb(225, 228, 232))),
        ),
    ])
}

fn active_title(title: &str, focused: bool) -> String {
    if focused {
        format!("{title} active")
    } else {
        title.to_string()
    }
}

fn panel_block(title: impl ToString, focused: bool) -> Block<'static> {
    let style = if focused {
        Style::default().fg(ui_color(Color::Rgb(121, 192, 255)))
    } else {
        Style::default().fg(ui_color(Color::Rgb(139, 148, 158)))
    };
    Block::default()
        .title(title.to_string())
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(style)
}

fn ui_color(color: Color) -> Color {
    if std::env::var_os("NO_COLOR").is_some() {
        Color::Reset
    } else {
        color
    }
}

fn path_label(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn modal_block(title: &str) -> Block<'_> {
    Block::default()
        .title(title.to_string())
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(ui_color(Color::Rgb(121, 192, 255))))
        .style(Style::default().bg(ui_color(Color::Rgb(13, 17, 23))))
}

fn kind_color(kind: ChangeKind) -> Color {
    match kind {
        ChangeKind::Added | ChangeKind::Untracked => ui_color(Color::Rgb(63, 185, 80)),
        ChangeKind::Deleted | ChangeKind::Conflict => ui_color(Color::Rgb(248, 81, 73)),
        ChangeKind::Renamed | ChangeKind::Copied => ui_color(Color::Rgb(121, 192, 255)),
        ChangeKind::Modified | ChangeKind::TypeChanged => ui_color(Color::Rgb(210, 168, 255)),
        ChangeKind::Ignored => ui_color(Color::Rgb(139, 148, 158)),
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(height.min(area.height)),
            Constraint::Min(0),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(width.min(area.width)),
            Constraint::Min(0),
        ])
        .split(vertical[1]);
    horizontal[1].inner(Margin {
        vertical: 0,
        horizontal: 0,
    })
}

fn truncate_middle(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    if max <= 3 {
        return "...".chars().take(max).collect();
    }
    let head_len = (max - 3) / 2;
    let tail_len = max - 3 - head_len;
    let head: String = text.chars().take(head_len).collect();
    let tail: String = text
        .chars()
        .rev()
        .take(tail_len)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}...{tail}")
}

fn truncate_end(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    if max <= 1 {
        return String::new();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('>');
    out
}

fn scroll_offset(value: usize) -> u16 {
    value.min(u16::MAX as usize) as u16
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::Command;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use tempfile::TempDir;

    use super::draw;
    use crate::app::App;
    use crate::git::GitCli;

    struct TestRepo {
        dir: TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let repo = Self { dir };
            let output = Command::new("git")
                .arg("-C")
                .arg(repo.dir.path())
                .args(["init", "-b", "main"])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            fs::write(repo.dir.path().join("notes.md"), "hello\nworld\n").unwrap();
            repo
        }
    }

    #[test]
    fn renders_core_layout_at_common_terminal_sizes() {
        let repo = TestRepo::new();
        let git = GitCli::discover(repo.dir.path()).unwrap();
        let app = App::new(git).unwrap();

        for (width, height) in [(120, 36), (80, 24), (60, 20)] {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
        }
    }
}
