//! 游戏日志窗口（独立子进程运行：`deepslate-iced --log-window [--light]`）。
//!
//! 主进程把游戏 stdout/stderr 逐行写入本进程 stdin（`O\t行` / `E\t行`）。
//! 缓冲超过 MAX_LINES 自动丢弃最旧行（压缩）；stdin 关闭（主进程退出）
//! 后显示结束横幅并在数秒后自动关闭窗口。

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};

use iced::{
    futures::{SinkExt, Stream},
    widget::{button, column, container, row, rule, scrollable, text, Space},
    Element, Font, Length, Size, Subscription, Task,
};
use tokio::sync::mpsc;

use crate::style;

const MAX_LINES: usize = 2000;
const AUTO_CLOSE_AFTER: Duration = Duration::from_secs(3);

struct LogWindow {
    dark: bool,
    lines: VecDeque<(bool, String)>,
    total: u64,
    dropped: u64,
    ended: bool,
    ended_at: Option<Instant>,
}

#[derive(Debug, Clone)]
enum Msg {
    Line { is_stderr: bool, line: String },
    Ended,
    Clear,
    Tick(Instant),
}

pub fn run(dark: bool) -> iced::Result {
    iced::application(boot(dark), update, view)
        .title("深板岩启动器 — 游戏日志")
        .theme(move |_state: &LogWindow| {
            if dark {
                style::theme_dark()
            } else {
                style::theme_light()
            }
        })
        .subscription(subscribe)
        .window_size(Size::new(840.0, 520.0))
        .run()
}

fn boot(dark: bool) -> impl Fn() -> (LogWindow, Task<Msg>) {
    move || {
        (
            LogWindow {
                dark,
                lines: VecDeque::new(),
                total: 0,
                dropped: 0,
                ended: false,
                ended_at: None,
            },
            Task::none(),
        )
    }
}

fn update(state: &mut LogWindow, message: Msg) -> Task<Msg> {
    match message {
        Msg::Line { is_stderr, line } => {
            state.total += 1;
            if state.lines.len() >= MAX_LINES {
                state.lines.pop_front();
                state.dropped += 1;
            }
            state.lines.push_back((is_stderr, line));
            Task::none()
        }
        Msg::Ended => {
            state.ended = true;
            state.ended_at = Some(Instant::now());
            Task::none()
        }
        Msg::Clear => {
            state.lines.clear();
            state.dropped = 0;
            Task::none()
        }
        Msg::Tick(now) => {
            if let Some(at) = state.ended_at {
                if now.duration_since(at) >= AUTO_CLOSE_AFTER {
                    return iced::exit();
                }
            }
            Task::none()
        }
    }
}

fn subscribe(state: &LogWindow) -> Subscription<Msg> {
    let mut subs = vec![Subscription::run(listen_stdin)];
    if state.ended {
        subs.push(iced::time::every(Duration::from_secs(1)).map(Msg::Tick));
    }
    Subscription::batch(subs)
}

/// 逐行读取本进程 stdin，解析 "O\t行" / "E\t行" 前缀
fn listen_stdin() -> impl Stream<Item = Msg> {
    iced::stream::channel(
        256,
        |mut output: iced::futures::channel::mpsc::Sender<Msg>| async move {
            let (tx, mut rx) = mpsc::channel::<Msg>(1024);
            std::thread::spawn(move || {
                let stdin = std::io::stdin();
                for line in BufReader::new(stdin.lock()).lines() {
                    let Ok(line) = line else { break };
                    let msg = if let Some(content) = line.strip_prefix("E\t") {
                        Msg::Line {
                            is_stderr: true,
                            line: content.to_string(),
                        }
                    } else if let Some(content) = line.strip_prefix("O\t") {
                        Msg::Line {
                            is_stderr: false,
                            line: content.to_string(),
                        }
                    } else {
                        Msg::Line {
                            is_stderr: false,
                            line,
                        }
                    };
                    if tx.blocking_send(msg).is_err() {
                        return;
                    }
                }
                let _ = tx.blocking_send(Msg::Ended);
            });
            while let Some(msg) = rx.recv().await {
                let _ = output.send(msg).await;
            }
        },
    )
}

fn view(state: &LogWindow) -> Element<'_, Msg> {
    let s = style::scheme_of(state.dark);

    let status_text = if state.ended {
        format!("日志已结束，{} 秒后自动关闭", AUTO_CLOSE_AFTER.as_secs())
    } else {
        "接收中…".to_string()
    };
    let status_color = if state.ended { s.text_faint } else { s.accent };

    let header = row![
        text("游戏日志").size(16).color(s.text_primary),
        Space::new().width(Length::Fill),
        text(format!(
            "共 {} 行 · 缓冲 {}/{} · 已压缩丢弃 {}",
            state.total,
            state.lines.len(),
            MAX_LINES,
            state.dropped
        ))
        .size(11)
        .color(s.text_faint),
        text(status_text).size(11).color(status_color),
        button(text("清空").size(12))
            .padding(5.0)
            .style(style::secondary)
            .on_press(Msg::Clear),
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);

    let body: Element<Msg> = if state.lines.is_empty() {
        text("等待日志…（启动游戏后，其 stdout/stderr 会显示在这里，最新在最上方）")
            .size(12)
            .color(s.text_faint)
            .into()
    } else {
        // 最新在最上方，无需滚动控制即可自动跟随
        let rows = state
            .lines
            .iter()
            .rev()
            .map(|(is_stderr, line)| {
                text(line.clone())
                    .size(12)
                    .font(Font::MONOSPACE)
                    .color(if *is_stderr { s.danger } else { s.text_muted })
                    .into()
            })
            .collect::<Vec<_>>();
        column(rows).spacing(1).into()
    };

    container(
        column![
            container(header).padding(12.0),
            rule::horizontal(1.0).style(style::rule_style),
            scrollable(container(body).padding(12.0)),
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(style::page)
    .into()
}
