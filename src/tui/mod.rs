mod render;

use std::io::{self, Stdout, Write};
use std::panic;
use std::process::Command;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::action::{key_to_action, key_to_text_action};
use crate::app::App;
use crate::config::{MouseMode, WatchMode};
use crate::model::{Modal, TerminalHandoff, TerminalHandoffAfter};
use crate::watcher::{FsWatcher, WatchEvent};

type Term = Terminal<CrosstermBackend<Stdout>>;
const POP_KEYBOARD_ENHANCEMENT: &[u8] = b"\x1b[<u";

pub fn run(app: &mut App) -> io::Result<()> {
    install_panic_hook();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    let mouse_enabled = app.config.ui.mouse != MouseMode::Never;
    if mouse_enabled {
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    } else {
        execute!(stdout, EnterAlternateScreen)?;
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(app, &mut terminal, mouse_enabled);

    let restore_result = restore_terminal(&mut terminal, mouse_enabled);
    result.and(restore_result)
}

fn run_loop(app: &mut App, terminal: &mut Term, mouse_enabled: bool) -> io::Result<()> {
    let mut last_refresh = Instant::now();
    let fallback_interval = app.config.git.auto_refresh_ms.map(Duration::from_millis);
    let mut periodic_interval = fallback_interval;
    let mut watcher = start_watcher(app, fallback_interval);
    if watcher.is_some() {
        periodic_interval = None;
    }
    let mut pending_refresh = false;
    loop {
        app.poll_background_refresh();
        if pending_refresh
            && matches!(app.state.modal, Modal::None | Modal::Help | Modal::Error)
            && app.request_background_refresh("")
        {
            pending_refresh = false;
            last_refresh = Instant::now();
        }
        if poll_watcher(
            app,
            &mut watcher,
            &mut periodic_interval,
            fallback_interval,
            &mut pending_refresh,
        ) {
            last_refresh = Instant::now();
        }
        if periodic_interval.is_some_and(|interval| last_refresh.elapsed() >= interval)
            && matches!(app.state.modal, Modal::None | Modal::Help | Modal::Error)
        {
            if !app.request_background_refresh("") {
                pending_refresh = true;
            }
            last_refresh = Instant::now();
        }

        terminal.draw(|frame| render::draw(frame, app))?;

        if app.state.should_quit {
            return Ok(());
        }

        if event::poll(Duration::from_millis(120))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let action = match app.state.modal {
                        Modal::Commit | Modal::Palette => key_to_text_action(key),
                        Modal::None | Modal::Help | Modal::Error | Modal::Confirm => {
                            key_to_action(key)
                        }
                    };
                    app.handle(action);
                    if let Some(handoff) = app.take_terminal_handoff() {
                        run_terminal_handoff(app, terminal, mouse_enabled, handoff)?;
                    }
                }
                Event::Mouse(mouse) if app.state.modal == Modal::None => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        focus_for_mouse(app, terminal.size()?.width, mouse.column);
                        app.handle(crate::action::Action::MoveUp);
                    }
                    MouseEventKind::ScrollDown => {
                        focus_for_mouse(app, terminal.size()?.width, mouse.column);
                        app.handle(crate::action::Action::MoveDown);
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        let width = terminal.size()?.width;
                        if width >= 92 && mouse.column < 40 {
                            app.handle(crate::action::Action::FocusChanges);
                            if mouse.row >= 2 {
                                let height = terminal.size()?.height.saturating_sub(2) as usize;
                                app.select_rail_visible_row((mouse.row - 2) as usize, height);
                            }
                        } else if width >= 92 {
                            app.handle(crate::action::Action::FocusDiff);
                        } else {
                            app.handle(crate::action::Action::FocusChanges);
                            if mouse.row >= 2 {
                                let height = terminal.size()?.height.saturating_sub(2) as usize;
                                app.select_rail_visible_row((mouse.row - 2) as usize, height);
                            }
                        }
                        if let Some(handoff) = app.take_terminal_handoff() {
                            run_terminal_handoff(app, terminal, mouse_enabled, handoff)?;
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

fn start_watcher(app: &mut App, periodic_interval: Option<Duration>) -> Option<FsWatcher> {
    if app.config.git.filesystem_watch == WatchMode::Never {
        return None;
    }
    let (root, git_dir, common_git_dir) = app.watch_paths();
    let interval = watcher_interval(periodic_interval);
    match FsWatcher::start(root, git_dir, common_git_dir, interval) {
        Ok(watcher) => Some(watcher),
        Err(err) => {
            if app.config.git.filesystem_watch == WatchMode::Always {
                app.state.status_message = format!("Filesystem watcher unavailable: {err}");
            }
            None
        }
    }
}

fn poll_watcher(
    app: &mut App,
    watcher: &mut Option<FsWatcher>,
    periodic_interval: &mut Option<Duration>,
    fallback_interval: Option<Duration>,
    pending_refresh: &mut bool,
) -> bool {
    let Some(active) = watcher.as_ref() else {
        if app.config.git.filesystem_watch != WatchMode::Never && periodic_interval.is_none() {
            *periodic_interval = Some(refresh_fallback_interval(fallback_interval));
        }
        return false;
    };

    match active.try_recv() {
        Ok(Some(WatchEvent::Changed)) => {
            if matches!(app.state.modal, Modal::None | Modal::Help | Modal::Error)
                && app.request_background_refresh("")
            {
                return true;
            }
            *pending_refresh = true;
        }
        Ok(Some(WatchEvent::Unavailable(reason))) => {
            app.state.status_message = format!("Filesystem watcher unavailable: {reason}");
            *watcher = None;
            if periodic_interval.is_none() {
                *periodic_interval = Some(refresh_fallback_interval(fallback_interval));
            }
        }
        Ok(None) => {}
        Err(_) => {
            app.state.status_message =
                "Filesystem watcher stopped; using periodic refresh".to_string();
            *watcher = None;
            if periodic_interval.is_none() {
                *periodic_interval = Some(refresh_fallback_interval(fallback_interval));
            }
        }
    }
    false
}

fn refresh_fallback_interval(configured: Option<Duration>) -> Duration {
    configured.unwrap_or_else(|| Duration::from_millis(2500))
}

fn watcher_interval(periodic_interval: Option<Duration>) -> Duration {
    periodic_interval
        .map(|interval| (interval / 2).clamp(Duration::from_millis(350), Duration::from_secs(2)))
        .unwrap_or_else(|| Duration::from_millis(750))
}

fn focus_for_mouse(app: &mut App, width: u16, column: u16) {
    if width < 92 {
        return;
    }
    if column >= 40 {
        app.handle(crate::action::Action::FocusDiff);
    } else {
        app.handle(crate::action::Action::FocusChanges);
    }
}

fn run_terminal_handoff(
    app: &mut App,
    terminal: &mut Term,
    mouse_enabled: bool,
    handoff: TerminalHandoff,
) -> io::Result<()> {
    restore_terminal(terminal, mouse_enabled)?;

    println!("gack: running {}", handoff.label);
    let pause_after = !matches!(handoff.after, TerminalHandoffAfter::Commit(_));
    let after = handoff.after;
    let status = Command::new(&handoff.command)
        .current_dir(&handoff.cwd)
        .args(&handoff.args)
        .status();
    let (success, detail) = match status {
        Ok(status) => {
            println!("gack: command exited with {status}");
            (status.success(), status.to_string())
        }
        Err(err) => {
            println!("gack: failed to run handoff: {err}");
            (false, err.to_string())
        }
    };
    if pause_after {
        println!("Press Enter to return to gack...");
        let mut line = String::new();
        let _ = io::stdin().read_line(&mut line);
    }

    enable_raw_mode()?;
    if mouse_enabled {
        execute!(
            terminal.backend_mut(),
            EnterAlternateScreen,
            EnableMouseCapture,
            Clear(ClearType::All)
        )?;
    } else {
        execute!(
            terminal.backend_mut(),
            EnterAlternateScreen,
            Clear(ClearType::All)
        )?;
    }
    terminal.clear()?;
    terminal.hide_cursor()?;
    app.complete_terminal_handoff(after, success, &detail);
    Ok(())
}

fn install_panic_hook() {
    let original = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let mut stdout = io::stdout();
        let _ = stdout.write_all(POP_KEYBOARD_ENHANCEMENT);
        let _ = execute!(stdout, DisableMouseCapture, LeaveAlternateScreen);
        let _ = disable_raw_mode();
        original(info);
    }));
}

fn restore_terminal(terminal: &mut Term, mouse_enabled: bool) -> io::Result<()> {
    let mut first_error = None;

    if let Err(err) = terminal.backend_mut().write_all(POP_KEYBOARD_ENHANCEMENT) {
        first_error.get_or_insert(err);
    }

    let leave_result = if mouse_enabled {
        execute!(
            terminal.backend_mut(),
            DisableMouseCapture,
            LeaveAlternateScreen
        )
    } else {
        execute!(terminal.backend_mut(), LeaveAlternateScreen)
    };
    if let Err(err) = leave_result {
        first_error.get_or_insert(err);
    }

    if let Err(err) = disable_raw_mode() {
        first_error.get_or_insert(err);
    }

    if let Err(err) = terminal.show_cursor() {
        first_error.get_or_insert(err);
    }

    match first_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}
