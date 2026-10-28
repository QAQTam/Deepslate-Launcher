use super::*;
use iced::widget::column;

// ── 视图 ─────────────────────────────────────────────────

pub(super) fn page_requires_outer_scroll(page: Page) -> bool {
    matches!(page, Page::Launch | Page::Friends | Page::Settings)
}

pub(super) fn view(app: &App) -> Element<'_, Message> {
    let s = style::scheme_of(app.dark_mode);

    let nav = |page: Page| {
        let active = app.page == page;
        button(text(page.label()).size(14))
            .width(Length::Fill)
            .padding(7.0)
            .style(style::nav(active))
            .on_press(Message::Navigate(page))
    };

    let busy = app.game_running || app.launching;
    let status_dot = if busy { s.accent } else { s.text_faint };

    let sidebar = container(
        column![
            row![
                text("◆").size(18).color(s.accent),
                column![
                    text("Deepslate").size(17).color(s.text_primary),
                    text("深板岩启动器").size(9).color(s.text_faint),
                ],
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            Space::new().height(Length::from(20)),
            nav(Page::Home),
            nav(Page::Launch),
            nav(Page::Friends),
            nav(Page::Maps),
            nav(Page::Server),
            nav(Page::Mods),
            nav(Page::Advanced),
            nav(Page::Settings),
            Space::new().height(Length::Fill),
            row![
                text("●").size(10).color(status_dot),
                text(game_state_label(app)).size(11).color(s.text_muted),
            ]
            .spacing(6),
        ]
        .spacing(4)
        .padding(14),
    )
    .width(Length::from(168))
    .height(Length::Fill)
    .style(style::sidebar);

    let page = page_view(app);
    let content_page: Element<'_, Message> = if page_requires_outer_scroll(app.page) {
        scrollable(page).height(Length::Fill).into()
    } else {
        page
    };
    let content = container(content_page)
        .padding(24)
        .width(Length::Fill)
        .height(Length::Fill);

    let main: Element<'_, Message> = container(row![sidebar, content])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(style::page)
        .into();
    if app.eula_required {
        stack(vec![main, opaque(eula_modal(app))]).into()
    } else {
        main
    }
}

/// 主窗口内的 EULA 模态。全窗遮罩 + `opaque`，底层控件不会接收点击。
fn eula_modal(app: &App) -> Element<'_, Message> {
    let s = style::scheme_of(app.dark_mode);
    let can_accept = app.eula_text.is_some()
        && !app.eula_loading
        && app.eula_error.is_none()
        && app.eula_scrolled_to_bottom
        && app.eula_checked;

    let body: Element<'_, Message> = if app.eula_loading {
        container(
            text("正在从 minecraft.net 获取官方 EULA…")
                .size(12)
                .color(s.text_muted),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center(Length::Fill)
        .into()
    } else if let Some(error) = &app.eula_error {
        column![
            text("无法获取 EULA 正文").size(14).color(s.danger),
            text(error.as_str()).size(12).color(s.text_muted),
            button(text("重新获取").size(12))
                .padding(6.0)
                .style(style::secondary)
                .on_press(Message::EulaRetry),
        ]
        .spacing(10)
        .into()
    } else if let Some(text_value) = &app.eula_text {
        scrollable(
            container(
                text(text_value.as_str())
                    .size(12)
                    .width(Length::Fill)
                    .color(s.text_primary),
            )
            .padding(4.0)
            .width(Length::Fill),
        )
        .height(Length::Fill)
        .on_scroll(|viewport| {
            let max = (viewport.content_bounds().height - viewport.bounds().height).max(0.0);
            Message::EulaScrolled(max <= 1.0 || viewport.absolute_offset().y >= max - 1.0)
        })
        .into()
    } else {
        container(text("等待 EULA 文本…").size(12).color(s.text_faint))
            .width(Length::Fill)
            .height(Length::Fill)
            .center(Length::Fill)
            .into()
    };

    let mut agree = checkbox(app.eula_checked)
        .label("我已阅读并同意 Minecraft EULA")
        .size(12)
        .style(style::checkbox_style);
    if app.eula_scrolled_to_bottom && !app.eula_loading && app.eula_error.is_none() {
        agree = agree.on_toggle(Message::EulaChecked);
    }

    let accept = {
        let b = button(text("同意并继续").size(13)).padding(8.0);
        if can_accept {
            b.style(style::primary).on_press(Message::EulaAccept)
        } else {
            b
        }
    };
    let hint = if app.eula_error.is_some() {
        "必须成功获取官方原文后才能同意；也可以重试。"
    } else if app.eula_loading {
        "正在获取官方文本…"
    } else if !app.eula_scrolled_to_bottom {
        "请先滚动阅读到正文底部，才能勾选同意。"
    } else if !app.eula_checked {
        "已阅读到底部，请勾选同意。"
    } else {
        "点击「同意并继续」后写入 eula.txt，并继续当前操作。"
    };

    let mut card = column![
        row![
            text("Minecraft EULA").size(20).color(s.text_primary),
            Space::new().width(Length::Fill),
            text("首次启动前必须同意").size(10).color(s.text_faint),
        ]
        .align_y(iced::Alignment::Center),
        text("正文直接来自 https://www.minecraft.net/en-us/eula，不手写、不截断。")
            .size(11)
            .color(s.text_muted),
    ]
    .spacing(8);
    if let Some(source) = &app.eula_source {
        card = card.push(text(source.as_str()).size(10).color(s.text_faint));
    }
    if let Some(warning) = &app.eula_warning {
        card = card.push(text(warning.as_str()).size(10).color(s.accent));
    }
    card = card.push(rule::horizontal(1.0).style(style::rule_style));
    card = card.push(body);
    card = card.push(rule::horizontal(1.0).style(style::rule_style));
    card = card.push(
        row![
            agree,
            Space::new().width(Length::Fill),
            button(text("取消").size(11))
                .padding(4.0)
                .style(style::secondary)
                .on_press(Message::EulaCancel),
            button(text("重新获取").size(11))
                .padding(4.0)
                .style(style::secondary)
                .on_press(Message::EulaRetry),
            accept,
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    );
    card = card.push(text(hint).size(10).color(s.text_faint));

    container(
        container(card)
            .width(Length::Fixed(820.0))
            .height(Length::Fill)
            .padding(22.0)
            .style(style::card),
    )
    .padding(24.0)
    .width(Length::Fill)
    .height(Length::Fill)
    .center(Length::Fill)
    .style(style::modal_scrim)
    .into()
}

/// 带标题的卡片区块
fn section_card<'a>(
    s: &style::Scheme,
    title: &str,
    body: Element<'a, Message>,
) -> Element<'a, Message> {
    container(column![text(title.to_string()).size(15).color(s.text_primary), body,].spacing(10))
        .padding(16)
        .style(style::card)
        .into()
}

/// 卡片入场：随页面进度 t 依次上浮淡入（delay 为错峰偏移，动画结束后零开销）
fn enter<'a>(t: f32, delay: f32, element: Element<'a, Message>) -> Element<'a, Message> {
    let ct = ((t - delay) / 0.5).clamp(0.0, 1.0);
    if ct >= 1.0 {
        return element;
    }
    let dy = (1.0 - ct) * 14.0;
    let scale = 1.0 - (1.0 - ct) * 0.03;
    float(element)
        .scale(scale)
        .translate(move |_viewport, _bounds| Vector::new(0.0, dy))
        .into()
}

fn game_state_label(app: &App) -> &'static str {
    if app.launching {
        "游戏启动中..."
    } else if app.game_running {
        "游戏运行中"
    } else {
        "空闲"
    }
}

fn page_view(app: &App) -> Element<'_, Message> {
    let t = app.page_anim.interpolate(0.0f32, 1.0, app.now);
    let page = match app.page {
        Page::Launch => launch_page(app, t),
        Page::Home => home_page(app, t),
        Page::Friends => friends_page(app, t),
        Page::Maps => maps_page(app, t),
        Page::Server => server_page(app, t),
        Page::Mods => mods_page(app, t),
        Page::Advanced => advanced_page(app, t),
        Page::Settings => settings_page(app, t),
    };
    // 整页轻微上浮入场
    if t >= 1.0 {
        page
    } else {
        let dy = (1.0 - t) * 10.0;
        float(page)
            .translate(move |_viewport, _bounds| Vector::new(0.0, dy))
            .into()
    }
}

/// Microsoft 登录：系统浏览器授权 + 粘贴回调 URL（webview 无法自动捕获 code）
fn ms_login_section<'a>(app: &'a App, s: &'static style::Scheme) -> Element<'a, Message> {
    let login_btn = {
        let b = button(text("完成登录").size(13)).padding(6.0);
        if app.ms_logging_in || app.ms_code_input.trim().is_empty() {
            b
        } else {
            b.style(style::primary).on_press(Message::MsLoginPressed)
        }
    };

    column![
        row![
            button(text("打开微软登录页").size(12))
                .padding(6.0)
                .style(style::secondary)
                .on_press(Message::MsAuthPressed),
            text("浏览器完成授权后，把跳转到的 URL 粘贴到下面")
                .size(11)
                .color(s.text_faint),
        ]
        .spacing(8),
        row![
            text_input("粘贴回调 URL（含 code=...）", &app.ms_code_input)
                .on_input(Message::MsCodeChanged)
                .on_submit(Message::MsLoginPressed)
                .style(style::text_field),
            login_btn,
        ]
        .spacing(8),
    ]
    .spacing(8)
    .into()
}

fn launch_page(app: &App, t: f32) -> Element<'_, Message> {
    let s = style::scheme_of(app.dark_mode);
    let tab_row = row([VersionTab::Release, VersionTab::Installed]
        .iter()
        .map(|tab| {
            let active = app.version_tab == *tab;
            button(text(tab.label()).size(13))
                .padding(7.0)
                .style(style::nav(active))
                .on_press(Message::TabChanged(*tab))
                .into()
        })
        .collect::<Vec<_>>())
    .spacing(6);

    let version_picker = pick_list(
        app.filtered.as_slice(),
        app.selected_version.as_ref(),
        Message::VersionSelected,
    )
    .placeholder("选择版本")
    .style(style::pick);

    let profile_picker = pick_list(
        app.profiles.clone(),
        app.selected_profile.as_ref(),
        Message::ProfileSelected,
    )
    .placeholder("选择档案")
    .style(style::pick);

    let new_profile_input = text_input("新建离线档案...", &app.new_profile_name)
        .on_input(Message::NewProfileNameChanged)
        .on_submit(Message::CreateProfile)
        .style(style::text_field);

    let java_picker = pick_list(
        app.java_list.clone(),
        app.selected_java.as_ref(),
        Message::JavaSelected,
    )
    .placeholder("选择 Java")
    .style(style::pick);

    // 下载按钮
    let download_btn = {
        let b = button(text("下载版本")).padding(8.0);
        let local = app
            .selected_version
            .as_ref()
            .map(|v| v.local)
            .unwrap_or(false);
        if app.downloading
            || app.fabric_installing
            || app.launching
            || app.selected_version.is_none()
            || local
        {
            b
        } else {
            b.style(style::primary).on_press(Message::DownloadPressed)
        }
    };

    // 启动/终止按钮
    let launch_btn = if app.game_running || app.launching {
        button(text(if app.active_managed {
            "保存并退出"
        } else {
            "终止游戏"
        }))
        .padding(8.0)
        .style(style::danger)
        .on_press(Message::KillPressed)
    } else {
        let ready = (app.config.enhanced_experience || app.selected_version.is_some())
            && app.selected_profile.is_some()
            && app.selected_java.is_some()
            && !app.fabric_installing;
        let b = button(text("启动游戏")).padding(8.0);
        if ready {
            b.style(style::primary).on_press(Message::LaunchPressed)
        } else {
            b
        }
    };

    // 下载组件时显示该阶段的实际进度；游戏进程启动后不显示旧进度。
    let progress_section: Element<Message> =
        if app.downloading || app.fabric_installing || (app.active_managed && app.launching) {
            match &app.progress {
                Some(p) => column![
                    progress_bar(0.0..=100.0, p.percent as f32).style(style::progress),
                    text(format!(
                        "{}/{}（{}%）— {}",
                        p.current, p.total, p.percent, p.current_file
                    ))
                    .size(12)
                    .color(s.text_muted),
                ]
                .spacing(8)
                .into(),
                None => text("准备中...").size(12).color(s.text_muted).into(),
            }
        } else {
            Space::new().height(Length::from(1)).into()
        };

    let selected_summary = row![
        text("档案:").size(12).color(s.text_muted),
        text(
            app.selected_profile
                .as_ref()
                .map(|p| p.name.as_str())
                .unwrap_or("未选择")
        )
        .size(12)
        .color(s.text_primary),
        text("版本:").size(12).color(s.text_muted),
        text(if app.config.enhanced_experience {
            "26.3（增强体验）".to_string()
        } else {
            app.selected_version
                .as_ref()
                .map(|v| v.to_string())
                .unwrap_or_else(|| "未选择".to_string())
        })
        .size(12)
        .color(s.text_primary),
    ]
    .spacing(8);

    let version_card = section_card(
        s,
        "版本",
        column![tab_row, version_picker].spacing(10).into(),
    );

    // Fabric 安装：选基准版本 + 加载器版本 → 下载 profile 库并写入 versions/
    let fabric_btn = {
        let b = button(text("安装 Fabric")).padding(8.0);
        let ready = app.fabric_game.is_some()
            && app.fabric_loader.is_some()
            && !app.fabric_installing
            && !app.fabric_loading
            && !app.downloading
            && !app.game_running
            && !app.launching;
        if ready {
            b.style(style::primary)
                .on_press(Message::FabricInstallPressed)
        } else {
            b
        }
    };
    let fabric_progress: Element<Message> = if app.fabric_installing {
        match &app.progress {
            Some(p) => column![
                progress_bar(0.0..=100.0, p.percent as f32).style(style::progress),
                text(format!(
                    "{}/{}（{}%）— {}",
                    p.current, p.total, p.percent, p.current_file
                ))
                .size(11)
                .color(s.text_muted),
            ]
            .spacing(6)
            .into(),
            None => text("准备安装...").size(11).color(s.text_muted).into(),
        }
    } else {
        Space::new().height(Length::from(1)).into()
    };
    let fabric_hint = if app.fabric_loading {
        "正在获取该版本可用的加载器..."
    } else if app.fabric_loaders.is_empty() {
        "选择基准版本后自动获取加载器列表"
    } else {
        "安装后出现在「已安装」；mod 放入实例目录的 mods/ 即可"
    };
    let fabric_card = section_card(
        s,
        "Fabric 加载器",
        column![
            row![
                pick_list(
                    app.releases.as_slice(),
                    app.fabric_game.as_ref(),
                    Message::FabricGameSelected,
                )
                .placeholder("基准 MC 版本")
                .style(style::pick),
                pick_list(
                    app.fabric_loaders.as_slice(),
                    app.fabric_loader.as_ref(),
                    Message::FabricLoaderSelected,
                )
                .placeholder(if app.fabric_loading {
                    "加载中..."
                } else {
                    "Fabric 版本"
                })
                .style(style::pick),
            ]
            .spacing(8),
            row![
                fabric_btn,
                button(text("刷新列表").size(12))
                    .padding(6.0)
                    .style(style::secondary)
                    .on_press(Message::RefreshFabricLoaders),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            fabric_progress,
            text(fabric_hint).size(11).color(s.text_faint),
        ]
        .spacing(8)
        .into(),
    );

    let profile_card = section_card(
        s,
        "档案",
        column![
            row![profile_picker, new_profile_input].spacing(8),
            ms_login_section(app, s),
        ]
        .spacing(10)
        .into(),
    );
    let java_card = section_card(
        s,
        "Java 运行时",
        row![
            java_picker,
            button(text("重新扫描").size(13))
                .padding(6.0)
                .style(style::secondary)
                .on_press(Message::RescanJava),
        ]
        .spacing(8)
        .into(),
    );
    let download_control: Element<Message> = if app.config.enhanced_experience {
        Space::new().width(0).into()
    } else {
        download_btn.into()
    };
    let status_text = if app.active_managed {
        let total = app
            .enhanced_started_at
            .map(|started| app.now.saturating_duration_since(started))
            .or(app.enhanced_startup_duration);
        let phase_elapsed = app
            .enhanced_phase
            .as_ref()
            .map(|(_, started)| app.now.saturating_duration_since(*started));
        match (total, phase_elapsed) {
            (Some(total), Some(phase)) => format!(
                "{} · 启动 {:.1}s · 当前阶段 {:.1}s",
                app.status,
                total.as_secs_f32(),
                phase.as_secs_f32()
            ),
            (Some(total), None) => format!("{} · 启动 {:.1}s", app.status, total.as_secs_f32()),
            _ => app.status.clone(),
        }
    } else {
        app.status.clone()
    };
    let actions_card = section_card(
        s,
        "启动",
        column![
            row![download_control, launch_btn].spacing(12),
            progress_section,
            selected_summary,
            text(status_text).size(12).color(s.text_muted),
        ]
        .spacing(12)
        .into(),
    );

    let server_btn = {
        let b = button(text("快速加入").size(13)).padding(7.0);
        if app.game_running
            || app.launching
            || app.fabric_installing
            || app.server_address.trim().is_empty()
        {
            b
        } else {
            b.style(style::primary).on_press(Message::QuickJoinServer)
        }
    };
    let recent_servers: Element<Message> = if app.config.recent_servers.is_empty() {
        text("暂无历史地址").size(11).color(s.text_faint).into()
    } else {
        row(app
            .config
            .recent_servers
            .iter()
            .take(4)
            .map(|address| {
                button(text(address.clone()).size(11))
                    .padding(4.0)
                    .style(style::secondary)
                    .on_press(Message::ServerAddressChanged(address.clone()))
                    .into()
            })
            .collect::<Vec<Element<Message>>>())
        .spacing(5)
        .into()
    };
    let saved_server_profiles: Element<Message> = if app.config.enhanced_experience {
        let saved_rows = app
            .config
            .saved_servers
            .iter()
            .take(8)
            .map(|server| {
                let status = app
                    .server_ping_status
                    .get(&server.address)
                    .cloned()
                    .unwrap_or_else(|| "未查询状态".into());
                column![
                    row![
                        button(text(format!("{} · {}", server.name, server.address)).size(11))
                            .padding(5)
                            .style(style::secondary)
                            .on_press(Message::ServerAddressChanged(server.address.clone())),
                        button(text("查询").size(10))
                            .padding(5)
                            .style(style::secondary)
                            .on_press(Message::PingServer(server.address.clone())),
                        button(text("移除").size(10))
                            .padding(5)
                            .style(style::danger)
                            .on_press(Message::RemoveServerProfile(server.address.clone())),
                    ]
                    .spacing(5)
                    .align_y(iced::Alignment::Center),
                    text(status).size(10).color(s.text_faint),
                ]
                .spacing(2)
                .into()
            })
            .collect::<Vec<Element<Message>>>();
        let saved_list: Element<Message> = if saved_rows.is_empty() {
            text("还没有收藏的服务器")
                .size(10)
                .color(s.text_faint)
                .into()
        } else {
            column(saved_rows).spacing(5).into()
        };
        column![
            row![
                text_input("收藏名称（可留空）", &app.server_profile_name)
                    .on_input(Message::ServerProfileNameChanged)
                    .on_submit(Message::SaveServerProfile)
                    .style(style::text_field),
                button(text("收藏地址").size(11))
                    .padding(6)
                    .style(style::secondary)
                    .on_press(Message::SaveServerProfile),
            ]
            .spacing(6),
            saved_list,
        ]
        .spacing(6)
        .into()
    } else {
        Space::new().height(0).into()
    };
    let server_card = section_card(
        s,
        "快速加入服务器",
        column![
            row![
                text_input("host 或 host:port", &app.server_address)
                    .on_input(Message::ServerAddressChanged)
                    .on_submit(Message::QuickJoinServer)
                    .style(style::text_field),
                server_btn,
            ]
            .spacing(8),
            row![text("最近:").size(11).color(s.text_muted), recent_servers]
                .spacing(6)
                .align_y(iced::Alignment::Center),
            saved_server_profiles,
            text(if app.config.enhanced_experience {
                "使用增强实例连接；退出服务器后关闭客户端。"
            } else {
                "启动后直接连接（需档案与 Java）。"
            })
            .size(11)
            .color(s.text_faint),
        ]
        .spacing(8)
        .into(),
    );

    // 双栏网格：左（版本/Fabric/档案）右（Java/启动）
    let left = if app.config.enhanced_experience {
        column![
            enter(
                t,
                0.00,
                section_card(
                    s,
                    "增强体验",
                    text("最低要求 Minecraft 26.3 · 当前已接入 26.3 · Fabric 与 Bridge 自动维护")
                        .size(12)
                        .color(s.text_muted)
                        .into()
                )
            ),
            enter(t, 0.16, profile_card)
        ]
    } else {
        column![
            enter(t, 0.00, version_card),
            enter(t, 0.08, fabric_card),
            enter(t, 0.16, profile_card)
        ]
    }
    .spacing(16)
    .width(Length::Fill);
    let right = column![
        enter(t, 0.24, java_card),
        enter(t, 0.36, actions_card),
        enter(t, 0.44, server_card),
    ]
    .spacing(16)
    .width(Length::Fill);

    column![
        text("启动").size(28).color(s.text_primary),
        text(format!(
            "游戏目录: {}",
            launcher::config::resolve_game_dir(&app.config).display()
        ))
        .size(11)
        .color(s.text_faint),
        rule::horizontal(1.0).style(style::rule_style),
        row![left, right].spacing(16),
    ]
    .spacing(16)
    .into()
}

// ── 首页 ─────────────────────────────────────────────────

fn info_row<'a>(s: &style::Scheme, label: &'a str, value: String) -> Element<'a, Message> {
    row![
        text(label).size(13).color(s.text_muted),
        text(value).size(13).color(s.text_primary),
    ]
    .spacing(8)
    .into()
}

fn home_page(app: &App, t: f32) -> Element<'_, Message> {
    let s = style::scheme_of(app.dark_mode);

    let profile_line = info_row(
        s,
        "档案:",
        app.selected_profile
            .as_ref()
            .map(|p| {
                format!(
                    "{}（{}）",
                    p.name,
                    if p.is_online() { "在线" } else { "离线" }
                )
            })
            .unwrap_or_else(|| "未选择".to_string()),
    );
    let version_line = info_row(
        s,
        "版本:",
        if app.config.enhanced_experience {
            "26.3（增强体验）".to_string()
        } else {
            app.selected_version
                .as_ref()
                .map(|v| v.to_string())
                .unwrap_or_else(|| "未选择".to_string())
        },
    );
    let java_line = info_row(
        s,
        "Java:",
        app.selected_java
            .as_ref()
            .map(|j| j.version.clone())
            .unwrap_or_else(|| "未找到".to_string()),
    );
    let state_line = info_row(s, "游戏:", game_state_label(app).to_string());
    let root_line = info_row(s, "根目录:", app.config.root_dir.clone());

    let status_card = section_card(
        s,
        "当前状态",
        column![profile_line, version_line, java_line, state_line, root_line,]
            .spacing(6)
            .into(),
    );

    let launch_btn = if app.game_running || app.launching {
        button(text(if app.active_managed {
            "保存并退出"
        } else {
            "终止游戏"
        }))
        .padding(8.0)
        .style(style::danger)
        .on_press(Message::KillPressed)
    } else {
        let b = button(text("启动游戏")).padding(8.0);
        let ready = (app.config.enhanced_experience || app.selected_version.is_some())
            && app.selected_profile.is_some()
            && app.selected_java.is_some()
            && !app.fabric_installing;
        if ready {
            b.style(style::primary).on_press(Message::LaunchPressed)
        } else {
            b
        }
    };

    let download_btn: Element<Message> = if app.config.enhanced_experience {
        button(text("关闭增强体验后可下载版本")).padding(8.0).into()
    } else {
        let download_btn = {
            let b = button(text("下载当前版本")).padding(8.0);
            let local = app
                .selected_version
                .as_ref()
                .map(|v| v.local)
                .unwrap_or(false);
            if app.downloading || app.fabric_installing || app.selected_version.is_none() || local {
                b
            } else {
                b.style(style::primary).on_press(Message::DownloadPressed)
            }
        };
        download_btn.into()
    };

    let continue_card = match app.saves.first() {
        Some(map) => section_card(
            s,
            "继续游戏",
            row![
                column![
                    text(map.name.clone()).size(18).color(s.text_primary),
                    text(format!(
                        "{} · 最后游玩 {}",
                        launcher::saves::summary_line(map),
                        map.last_played_text()
                    ))
                    .size(11)
                    .color(s.text_muted),
                ]
                .spacing(5),
                Space::new().width(Length::Fill),
                if app.game_running || app.launching {
                    button(text("进入").size(13)).padding(8.0)
                } else {
                    button(
                        text(
                            if app.pending_world_enter.as_deref() == Some(map.id.as_str()) {
                                "确认进入？"
                            } else {
                                "进入"
                            },
                        )
                        .size(13),
                    )
                    .padding(8.0)
                    .style(style::primary)
                    .on_press(Message::SaveEnter(map.id.clone()))
                },
            ]
            .align_y(iced::Alignment::Center)
            .spacing(12)
            .into(),
        ),
        None => section_card(
            s,
            "世界",
            row![
                text("地图库还是空的").size(13).color(s.text_muted),
                Space::new().width(Length::Fill),
                button(text("打开世界库").size(13))
                    .padding(8.0)
                    .style(style::primary)
                    .on_press(Message::Navigate(Page::Maps)),
            ]
            .align_y(iced::Alignment::Center)
            .spacing(12)
            .into(),
        ),
    };

    column![
        text("首页").size(28).color(s.text_primary),
        rule::horizontal(1.0).style(style::rule_style),
        enter(t, 0.0, status_card),
        enter(t, 0.06, continue_card),
        enter(t, 0.12, row![launch_btn, download_btn,].spacing(12).into()),
        text("提示：版本 / 档案 / Java 在「启动」页选择；世界与地图库在「世界」页管理。")
            .size(12)
            .color(s.text_faint),
        text(app.status.as_str()).size(12).color(s.text_muted),
    ]
    .spacing(16)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

// ── 世界/地图库页 ─────────────────────────────────────────

fn maps_page(app: &App, t: f32) -> Element<'_, Message> {
    let s = style::scheme_of(app.dark_mode);
    let maps_dir = launcher::maps::root(&app.config);

    let header = column![
        text("世界").size(28).color(s.text_primary),
        text(format!(
            "地图库: {}  ·  {} 个世界",
            maps_dir.display(),
            app.saves.len()
        ))
        .size(11)
        .color(s.text_faint),
        rule::horizontal(1.0).style(style::rule_style),
    ]
    .spacing(8);

    let toolbar = row![
        button(text("刷新").size(13))
            .padding(7.0)
            .style(style::secondary)
            .on_press(Message::RefreshSaves),
        button(text("打开地图库").size(13))
            .padding(7.0)
            .style(style::secondary)
            .on_press(Message::OpenSavesFolder),
        button(
            text(if app.confirm_empty_trash {
                "确认永久清空？"
            } else {
                "清空回收站"
            })
            .size(13)
        )
        .padding(7.0)
        .style(style::secondary)
        .on_press(Message::EmptyTrash),
        Space::new().width(Length::Fill),
        text(if app.config.enhanced_experience {
            "增强体验使用 Bridge 进入世界"
        } else {
            "「进入」使用 --quickPlaySingleplayer（Minecraft 1.20+）"
        })
        .size(11)
        .color(s.text_faint),
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);

    let import_row = row![
        text_input("地图 zip 或世界文件夹路径", &app.map_import_path)
            .on_input(Message::MapImportPathChanged)
            .on_submit(Message::ImportMap)
            .style(style::text_field),
        button(text("导入").size(13))
            .padding(7.0)
            .style(style::primary)
            .on_press(Message::ImportMap),
        button(text("选 zip").size(12))
            .padding(7.0)
            .style(style::secondary)
            .on_press(Message::PickMapFile),
        button(text("选文件夹").size(12))
            .padding(7.0)
            .style(style::secondary)
            .on_press(Message::PickMapFolder),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let list: Element<Message> = if app.saves.is_empty() {
        column![
            text("地图库还是空的").size(13).color(s.text_muted),
            text("可以在下面生成一个新世界，或导入 zip / 世界文件夹。")
                .size(11)
                .color(s.text_faint),
        ]
        .spacing(6)
        .into()
    } else {
        let busy = app.game_running || app.launching;
        let rows = app
            .saves
            .iter()
            .map(|world| {
                let renaming = app.renaming_save.as_deref() == Some(world.id.as_str());
                let deleting = app.pending_save_delete.as_deref() == Some(world.id.as_str());
                let restoring = app.pending_map_restore.as_deref() == Some(world.id.as_str());

                let enter_btn = {
                    let b = button(
                        text(
                            if app.pending_world_enter.as_deref() == Some(world.id.as_str()) {
                                "确认进入？"
                            } else {
                                "进入"
                            },
                        )
                        .size(12),
                    )
                    .padding(5.0);
                    if busy {
                        b
                    } else {
                        b.style(style::primary)
                            .on_press(Message::SaveEnter(world.id.clone()))
                    }
                };
                let rename_btn = button(text("重命名").size(12))
                    .padding(5.0)
                    .style(style::secondary)
                    .on_press(Message::SaveRenameStart(world.id.clone()));
                let copy_btn = button(text("复制").size(12))
                    .padding(5.0)
                    .style(style::secondary)
                    .on_press(Message::SaveDuplicate(world.id.clone()));
                let export_btn = button(text("导出").size(12))
                    .padding(5.0)
                    .style(style::secondary)
                    .on_press(Message::ExportMap(world.id.clone()));
                let export_as_btn = button(text("导出为...").size(12))
                    .padding(5.0)
                    .style(style::secondary)
                    .on_press(Message::ExportMapAs(world.id.clone()));
                let backup_btn = button(text("备份").size(12))
                    .padding(5.0)
                    .style(style::secondary)
                    .on_press(Message::MapBackup(world.id.clone()));
                let restore_btn = {
                    let b = button(
                        text(if restoring {
                            "确认恢复？"
                        } else {
                            "恢复"
                        })
                        .size(12),
                    )
                    .padding(5.0);
                    let b = if restoring {
                        b.style(style::danger)
                    } else {
                        b.style(style::secondary)
                    };
                    b.on_press(Message::MapRestore(world.id.clone()))
                };
                let delete_btn = {
                    let b = button(
                        text(if deleting {
                            "确认删除？"
                        } else {
                            "删除"
                        })
                        .size(12),
                    )
                    .padding(5.0);
                    let b = if deleting {
                        b.style(style::danger)
                    } else {
                        b.style(style::secondary)
                    };
                    b.on_press(Message::SaveDeletePressed(world.id.clone()))
                };

                let mut badges: Vec<Element<Message>> = Vec::new();
                if world.hardcore {
                    badges.push(text("极限").size(10).color(s.accent).into());
                }
                if world.cheats {
                    badges.push(text("作弊").size(10).color(s.text_faint).into());
                }
                if world.icon.is_some() {
                    badges.push(text("有图标").size(10).color(s.text_faint).into());
                }
                badges.push(
                    text(world.source_label())
                        .size(10)
                        .color(s.text_faint)
                        .into(),
                );

                let mut body = column![
                    row![
                        text(world.name.clone()).size(15).color(s.text_primary),
                        row(badges).spacing(6),
                        Space::new().width(Length::Fill),
                        enter_btn,
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                    text(launcher::saves::summary_line(world))
                        .size(11)
                        .color(s.text_muted),
                    text(format!("{} · 文件夹 {}", world.size_text(), world.id))
                        .size(10)
                        .color(s.text_faint),
                    row![
                        rename_btn,
                        copy_btn,
                        export_btn,
                        export_as_btn,
                        backup_btn,
                        restore_btn,
                        delete_btn,
                    ]
                    .spacing(6),
                ]
                .spacing(6);

                if renaming {
                    body = body.push(
                        row![
                            text_input("新的世界名称", &app.save_name_input)
                                .on_input(Message::SaveRenameChanged)
                                .on_submit(Message::SaveRenameConfirm)
                                .style(style::text_field),
                            button(text("确认").size(12))
                                .padding(5.0)
                                .style(style::primary)
                                .on_press(Message::SaveRenameConfirm),
                            button(text("取消").size(12))
                                .padding(5.0)
                                .style(style::secondary)
                                .on_press(Message::SaveRenameCancel),
                        ]
                        .spacing(8),
                    );
                }
                if deleting {
                    body = body.push(
                        text("再次点击「确认删除？」将移动到 maps/.deepslate-trash/（可恢复）")
                            .size(11)
                            .color(s.danger),
                    );
                }

                container(body)
                    .padding(12)
                    .width(Length::Fill)
                    .style(style::card)
                    .into()
            })
            .collect::<Vec<_>>();

        scrollable(column(rows).spacing(10))
            .height(Length::Fill)
            .into()
    };

    let gamemode_picker = pick_list(
        vec!["survival".to_string(), "creative".to_string()],
        Some(app.new_world_gamemode.clone()),
        Message::NewWorldGameMode,
    )
    .style(style::pick);
    let difficulty_picker = pick_list(
        vec![
            "和平".to_string(),
            "简单".to_string(),
            "普通".to_string(),
            "困难".to_string(),
        ],
        Some(
            match app.new_world_difficulty.as_str() {
                "peaceful" => "和平",
                "easy" => "简单",
                "hard" => "困难",
                _ => "普通",
            }
            .to_string(),
        ),
        |label: String| {
            Message::NewWorldDifficulty(
                match label.as_str() {
                    "和平" => "peaceful",
                    "简单" => "easy",
                    "困难" => "hard",
                    _ => "normal",
                }
                .to_string(),
            )
        },
    )
    .style(style::pick);
    let enhanced_world_options: Element<Message> = if app.config.enhanced_experience {
        column![
            row![
                text("难度").size(12).color(s.text_muted),
                difficulty_picker,
                checkbox(app.new_world_keep_inventory)
                    .label("死亡保留物品")
                    .on_toggle(Message::NewWorldKeepInventory)
                    .size(12)
                    .style(style::checkbox_style),
                checkbox(app.new_world_mob_griefing)
                    .label("生物破坏方块")
                    .on_toggle(Message::NewWorldMobGriefing)
                    .size(12)
                    .style(style::checkbox_style),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            row![
                checkbox(app.new_world_daylight_cycle)
                    .label("昼夜交替")
                    .on_toggle(Message::NewWorldDaylightCycle)
                    .size(12)
                    .style(style::checkbox_style),
                checkbox(app.new_world_weather_cycle)
                    .label("天气变化")
                    .on_toggle(Message::NewWorldWeatherCycle)
                    .size(12)
                    .style(style::checkbox_style),
                checkbox(app.new_world_hostile_mobs)
                    .label("生成敌对生物")
                    .on_toggle(Message::NewWorldHostileMobs)
                    .size(12)
                    .style(style::checkbox_style),
                checkbox(app.new_world_pvp)
                    .label("允许玩家互相伤害")
                    .on_toggle(Message::NewWorldPvp)
                    .size(12)
                    .style(style::checkbox_style),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(8)
        .into()
    } else {
        Space::new().height(0).into()
    };
    let create_btn = {
        let b = button(
            text(if app.config.enhanced_experience {
                "创建并进入"
            } else {
                "生成世界"
            })
            .size(13),
        )
        .padding(7.0);
        let ready = !app.new_world_creating
            && !app.game_running
            && (app.config.enhanced_experience || app.selected_version.is_some())
            && app.selected_java.is_some()
            && !app.new_world_name.trim().is_empty();
        if ready {
            b.style(style::primary).on_press(Message::CreateWorld)
        } else {
            b
        }
    };
    let new_world_card = section_card(
        s,
        "新建世界",
        column![
            row![
                text_input("世界名称", &app.new_world_name)
                    .on_input(Message::NewWorldNameChanged)
                    .style(style::text_field),
                text_input("种子（留空随机）", &app.new_world_seed)
                    .on_input(Message::NewWorldSeedChanged)
                    .style(style::text_field),
                gamemode_picker,
                checkbox(app.new_world_flat)
                    .label("超平坦")
                    .on_toggle(Message::NewWorldFlat)
                    .size(12)
                    .style(style::checkbox_style),
                checkbox(app.new_world_cheats)
                    .label("允许作弊")
                    .on_toggle(Message::NewWorldCheats)
                    .size(12)
                    .style(style::checkbox_style),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            enhanced_world_options,
            row![
                text(if app.eula_accepted {
                    "Minecraft EULA 已同意"
                } else {
                    "生成前会要求同意 Minecraft EULA"
                })
                .size(11)
                .color(s.text_faint),
                Space::new().width(Length::Fill),
                create_btn,
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            text(if app.config.enhanced_experience {
                "使用 Fabric 客户端创建并进入世界；首次需要安装组件。"
            } else {
                "使用后台原版服务端生成世界；首次会下载约 60MB server.jar，需要 Java 与 EULA 同意。"
            })
            .size(10)
            .color(s.text_faint),
        ]
        .spacing(8)
        .into(),
    );

    let pending_card: Element<Message> = if app.pending_creations.is_empty() {
        Space::new().height(0).into()
    } else {
        let rows = app
            .pending_creations
            .iter()
            .map(|world| {
                let status = world
                    .problem
                    .as_deref()
                    .unwrap_or("存档可读取；恢复后请检查世界内容");
                let mut recover = button(text("恢复到地图库").size(12)).padding(6.0);
                if world.problem.is_none() && !app.game_running && !app.launching {
                    recover = recover
                        .style(style::secondary)
                        .on_press(Message::RecoverCreation(world.id.clone()));
                }
                row![
                    column![
                        text(world.name.clone()).size(13).color(s.text_primary),
                        text(format!("{} · {}", world.id, status))
                            .size(11)
                            .color(s.text_muted)
                    ]
                    .spacing(4),
                    Space::new().width(Length::Fill),
                    recover
                ]
                .spacing(10)
                .into()
            })
            .collect::<Vec<Element<Message>>>();
        section_card(s, "待恢复的世界", column(rows).spacing(10).into())
    };
    column![
        enter(t, 0.0, header.into()),
        enter(t, 0.08, toolbar.into()),
        enter(t, 0.10, import_row.into()),
        enter(t, 0.14, new_world_card),
        enter(t, 0.18, pending_card),
        enter(t, 0.22, list),
        text(app.status.as_str()).size(12).color(s.text_muted),
    ]
    .spacing(14)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn mods_page(app: &App, t: f32) -> Element<'_, Message> {
    let s = style::scheme_of(app.dark_mode);
    let version_label = if app.config.enhanced_experience {
        "26.3 (增强体验)".to_string()
    } else {
        app.selected_version
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "未选择版本".to_string())
    };
    let has_loader = app.config.enhanced_experience
        || app
            .selected_version
            .as_ref()
            .map(|v| v.loader.is_some())
            .unwrap_or(false);
    let dir = app
        .mods_version
        .as_ref()
        .map(|v| launcher::mods::mods_dir(&app.config, v))
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "（先选择版本）".to_string());
    let enabled_count = app.mods.iter().filter(|m| m.enabled).count();

    let header = column![
        text("模组").size(28).color(s.text_primary),
        text(format!(
            "版本: {version_label}  ·  {enabled_count}/{} 启用  ·  {dir}",
            app.mods.len()
        ))
        .size(11)
        .color(s.text_faint),
        if !has_loader {
            text("当前版本没有 Fabric/Quilt 等加载器，游戏不会读取 mods/；请选择已安装的 Fabric 版本。")
                .size(11)
                .color(s.danger)
        } else {
            text("启用/禁用通过重命名 .jar 与 .jar.disabled 实现，重启游戏后生效。")
                .size(11)
                .color(s.text_faint)
        },
        rule::horizontal(1.0).style(style::rule_style),
    ]
    .spacing(8);

    let toolbar = row![
        button(text("刷新").size(13))
            .padding(7.0)
            .style(style::secondary)
            .on_press(Message::RefreshMods),
        button(text("打开 mods 目录").size(13))
            .padding(7.0)
            .style(style::secondary)
            .on_press(Message::OpenModsFolder),
        button(text("清空模组回收站").size(13))
            .padding(7.0)
            .style(style::secondary)
            .on_press(Message::EmptyModTrash),
        Space::new().width(Length::Fill),
        text("支持 fabric.mod.json 的 id/name/version/authors/environment")
            .size(10)
            .color(s.text_faint),
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);

    let list: Element<Message> =
        if !app.config.enhanced_experience && app.selected_version.is_none() {
            column![
                text("请先在「启动」页选择版本")
                    .size(13)
                    .color(s.text_muted),
                button(text("前往启动页").size(13))
                    .padding(7.0)
                    .style(style::primary)
                    .on_press(Message::Navigate(Page::Launch)),
            ]
            .spacing(12)
            .into()
        } else if app.mods.is_empty() {
            column![
                text("mods/ 里还没有模组").size(13).color(s.text_muted),
                text("把 Fabric 模组 jar 放进上面的目录，然后点「刷新」。")
                    .size(11)
                    .color(s.text_faint),
                button(text("打开 mods 目录").size(13))
                    .padding(7.0)
                    .style(style::primary)
                    .on_press(Message::OpenModsFolder),
            ]
            .spacing(10)
            .into()
        } else {
            let rows = app
                .mods
                .iter()
                .map(|mod_info| {
                    let deleting =
                        app.pending_mod_delete.as_deref() == Some(mod_info.file_name.as_str());
                    let toggle_label = if mod_info.enabled { "禁用" } else { "启用" };
                    let toggle = button(text(toggle_label).size(12))
                        .padding(5.0)
                        .style(if mod_info.enabled {
                            style::secondary
                        } else {
                            style::primary
                        })
                        .on_press(Message::ToggleMod(
                            mod_info.file_name.clone(),
                            !mod_info.enabled,
                        ));
                    let delete = {
                        let b = button(
                            text(if deleting {
                                "确认删除？"
                            } else {
                                "删除"
                            })
                            .size(12),
                        )
                        .padding(5.0);
                        let b = if deleting {
                            b.style(style::danger)
                        } else {
                            b.style(style::secondary)
                        };
                        b.on_press(Message::ModDeletePressed(mod_info.file_name.clone()))
                    };

                    let mut details = vec![
                        format!(
                            "{}  ·  {}",
                            mod_info.size_text(),
                            mod_info.environment_label()
                        ),
                        mod_info.file_name.clone(),
                    ];
                    if !mod_info.authors.is_empty() {
                        details.insert(0, format!("作者: {}", mod_info.authors.join(", ")));
                    }
                    if !mod_info.id.is_empty() {
                        details.push(format!("id: {}", mod_info.id));
                    }

                    let enabled_badge: Element<Message> = if mod_info.enabled {
                        text("已启用").size(10).color(s.accent).into()
                    } else {
                        text("已禁用").size(10).color(s.text_faint).into()
                    };
                    let mut body = column![
                        row![
                            text(mod_info.name.clone()).size(15).color(s.text_primary),
                            text(if mod_info.version.is_empty() {
                                String::new()
                            } else {
                                format!("v{}", mod_info.version)
                            })
                            .size(11)
                            .color(s.text_faint),
                            enabled_badge,
                            Space::new().width(Length::Fill),
                            toggle,
                            delete,
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center),
                        text(details.join("  ·  ")).size(10).color(s.text_faint),
                    ]
                    .spacing(6);
                    if !mod_info.description.is_empty() {
                        body = body.push(
                            text(mod_info.description.clone())
                                .size(11)
                                .color(s.text_muted),
                        );
                    }
                    if let Some(error) = &mod_info.metadata_error {
                        body = body.push(text(error.clone()).size(10).color(s.danger));
                    }
                    if deleting {
                        body = body.push(
                            text("再次点击「确认删除？」移动到 mods/.deepslate-trash/（可恢复）")
                                .size(10)
                                .color(s.danger),
                        );
                    }
                    container(body)
                        .padding(12)
                        .width(Length::Fill)
                        .style(style::card)
                        .into()
                })
                .collect::<Vec<_>>();
            scrollable(column(rows).spacing(10))
                .height(Length::Fill)
                .into()
        };

    column![
        enter(t, 0.0, header.into()),
        enter(t, 0.08, toolbar.into()),
        enter(t, 0.16, list),
        text(app.status.as_str()).size(12).color(s.text_muted),
    ]
    .spacing(14)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn friends_page(app: &App, t: f32) -> Element<'_, Message> {
    let s = style::scheme_of(app.dark_mode);
    let account = app
        .selected_profile
        .as_ref()
        .map(|profile| profile.name.as_str())
        .unwrap_or("尚未选择账号");
    let service_state = match app.friends.service_state.as_str() {
        "success" => "已连接 Mojang 好友服务",
        "loading" => "正在同步 Mojang 好友服务",
        "upgrade_needed" => "需要更新 Minecraft 才能使用好友服务",
        "connection_issue" => "好友服务连接异常",
        "user_may_lack_active_profile" => "当前账号没有可用的 Java 玩家档案",
        "unauthorized" => "当前账号无法访问好友服务",
        "temporary_unavailable" => "好友服务暂时不可用",
        "generic_error" => "好友服务发生错误",
        _ => "尚未从 Minecraft 同步",
    };
    let sync_label = if app.friends.updated_at == 0 {
        "等待从增强模式世界或服务器首次同步"
    } else {
        "好友数据来自 Minecraft 官方好友服务"
    };
    let settings_label = if app.friends.updated_at == 0 {
        "好友列表设置将在首次同步后显示".to_string()
    } else {
        format!(
            "好友列表{} · 好友申请{}",
            if app.friends.enabled {
                "已开启"
            } else {
                "已关闭"
            },
            if app.friends.allow_requests {
                "已开放"
            } else {
                "已关闭"
            }
        )
    };
    let overview = section_card(
        s,
        "Minecraft 官方好友服务",
        column![
            row![
                text(format!("当前账号: {account}")).size(12).color(s.text_muted),
                Space::new().width(Length::Fill),
                text(format!("{} 位好友", app.friends.friends.len()))
                    .size(11)
                    .color(s.text_faint),
            ]
            .align_y(iced::Alignment::Center),
            text(service_state).size(13).color(s.text_primary),
            text(sync_label).size(11).color(s.text_faint),
            text(settings_label).size(11).color(s.text_faint),
            text(format!(
                "申请：收到 {} · 已发送 {}",
                app.friends.incoming_requests.len(),
                app.friends.outgoing_requests.len()
            ))
            .size(11)
            .color(s.text_faint),
            text("在游戏内 Minecraft 官方好友页面发送、接受或撤回好友申请；启动器展示最近一次同步的列表与请求。")
                .size(11)
                .color(s.text_muted),
        ]
        .spacing(8)
        .into(),
    );

    let list_body: Element<'_, Message> = if app.friends_loading {
        text("正在读取此账号的本地好友快照…")
            .size(12)
            .color(s.text_muted)
            .into()
    } else if let Some(error) = &app.friend_error {
        text(format!("无法读取好友快照: {error}"))
            .size(12)
            .color(s.danger)
            .into()
    } else if app.selected_profile.is_none() {
        text("请先在「启动」页选择或创建 Minecraft 账号档案。")
            .size(12)
            .color(s.text_muted)
            .into()
    } else if app.friends.friends.is_empty() {
        text("此账号当前没有已同步的好友。进入增强模式世界或服务器后，好友列表会从 Minecraft 26.3 官方服务同步。")
            .size(12)
            .color(s.text_muted)
            .into()
    } else {
        column(app.friends.friends.iter().map(|friend| {
            let id_hint = friend.profile_id.chars().take(8).collect::<String>();
            let presence = friend
                .presence
                .as_deref()
                .map(friend_presence_label)
                .unwrap_or("状态未知");
            container(
                row![
                    column![
                        text(friend.name.clone()).size(13).color(s.text_primary),
                        text(format!("Minecraft 玩家 · {id_hint}"))
                            .size(10)
                            .color(s.text_faint),
                    ]
                    .spacing(3),
                    Space::new().width(Length::Fill),
                    text(presence).size(10).color(s.text_muted),
                ]
                .spacing(9)
                .align_y(iced::Alignment::Center),
            )
            .padding(10)
            .style(style::card)
            .into()
        }))
        .spacing(8)
        .into()
    };
    let list_card = section_card(s, "好友", list_body);
    let incoming = friends_request_card(
        s,
        "收到的申请",
        &app.friends.incoming_requests,
        "暂无收到的好友申请。",
    );
    let outgoing = friends_request_card(
        s,
        "已发送的申请",
        &app.friends.outgoing_requests,
        "暂无待处理的好友申请。",
    );

    column![
        enter(
            t,
            0.0,
            column![
                text("好友").size(28).color(s.text_primary),
                text("Minecraft 原生好友关系同步到启动器；邀请联机将在好友服务之外单独扩展。")
                    .size(12)
                    .color(s.text_muted),
                rule::horizontal(1.0).style(style::rule_style),
            ]
            .spacing(8)
            .into(),
        ),
        enter(t, 0.08, overview),
        enter(t, 0.16, list_card),
        enter(t, 0.24, incoming),
        enter(t, 0.32, outgoing),
    ]
    .spacing(14)
    .width(Length::Fill)
    .into()
}

fn friend_presence_label(status: &str) -> &'static str {
    match status {
        "online" => "在线",
        "playing_offline" => "单人游戏",
        "playing_realms" => "正在游玩 Realms",
        "playing_server" => "正在游玩服务器",
        "playing_hosted_server" => "正在游玩托管服务器",
        "offline" => "离线",
        _ => "状态未知",
    }
}

fn friends_request_card<'a>(
    s: &'a style::Scheme,
    title: &'a str,
    entries: &'a [launcher::friends::FriendEntry],
    empty_label: &'a str,
) -> Element<'a, Message> {
    let body: Element<'_, Message> = if entries.is_empty() {
        text(empty_label).size(12).color(s.text_muted).into()
    } else {
        column(entries.iter().map(|friend| {
            let id_hint = friend.profile_id.chars().take(8).collect::<String>();
            container(
                row![
                    column![
                        text(friend.name.clone()).size(13).color(s.text_primary),
                        text(format!("Minecraft 玩家 · {id_hint}"))
                            .size(10)
                            .color(s.text_faint),
                    ]
                    .spacing(3),
                    Space::new().width(Length::Fill),
                    text("在游戏内管理").size(10).color(s.text_muted),
                ]
                .spacing(9)
                .align_y(iced::Alignment::Center),
            )
            .padding(10)
            .style(style::card)
            .into()
        }))
        .spacing(8)
        .into()
    };
    section_card(s, title, body)
}

fn server_page(app: &App, t: f32) -> Element<'_, Message> {
    let s = style::scheme_of(app.dark_mode);
    let version_label = if app.config.enhanced_experience {
        "26.3".to_string()
    } else {
        app.selected_version
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "未选择版本".to_string())
    };
    let port = app.config.server_port;
    let state_text = if app.server_ready {
        format!("运行中 · 127.0.0.1:{port} 已就绪")
    } else if app.server_running {
        "运行中 · 等待就绪...".to_string()
    } else {
        "未运行".to_string()
    };

    let memory_picker = pick_list(
        vec![1024u32, 2048, 4096, 8192],
        Some(app.config.server_memory_mb),
        Message::ServerMemorySelected,
    )
    .placeholder("内存")
    .style(style::pick);

    let install_btn = {
        let b = button(text("安装/校验服务端").size(13)).padding(7.0);
        if app.server_busy {
            b
        } else {
            b.style(style::secondary).on_press(Message::ServerInstall)
        }
    };
    let start_btn = {
        let b = button(text("启动服务器").size(13)).padding(7.0);
        if app.server_busy || app.server_running {
            b
        } else {
            b.style(style::primary).on_press(Message::ServerStart)
        }
    };
    let join_btn = {
        let b = button(text("启动并加入").size(13)).padding(7.0);
        let ready = !app.server_busy
            && !app.server_running
            && app.selected_profile.is_some()
            && app.selected_java.is_some();
        if ready {
            b.style(style::primary)
                .on_press(Message::ServerStartAndJoin)
        } else {
            b
        }
    };
    let stop_btn = {
        let b = button(text("停止服务器").size(13)).padding(7.0);
        if app.server_running && !app.server_busy {
            b.style(style::danger).on_press(Message::ServerStop)
        } else {
            b
        }
    };
    let join_now_btn = {
        let b = button(text("加入服务器").size(13)).padding(7.0);
        if app.server_ready && !app.server_busy && !app.game_running {
            b.style(style::primary).on_press(Message::ServerJoinNow)
        } else {
            b
        }
    };

    let server_dir = if app.config.enhanced_experience {
        launcher::server::server_dir(&app.config, launcher::enhanced::MINECRAFT)
            .display()
            .to_string()
    } else {
        app.selected_version
            .as_ref()
            .map(|v| launcher::server::server_dir(&app.config, &v.id))
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "（先选择版本）".to_string())
    };

    let controls = section_card(
        s,
        "本地服务端",
        column![
            text(format!("版本: {version_label}  ·  状态: {state_text}"))
                .size(12)
                .color(s.text_muted),
            row![
                text("端口").size(12).color(s.text_muted),
                text_input("25565", &app.server_port_input)
                    .on_input(Message::ServerPortChanged)
                    .width(Length::from(100))
                    .style(style::text_field),
                text("内存").size(12).color(s.text_muted),
                memory_picker,
                checkbox(app.config.server_online_mode)
                    .label("正版验证")
                    .on_toggle(Message::ServerOnlineMode)
                    .size(12)
                    .style(style::checkbox_style),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            row![
                text(if app.eula_accepted {
                    "Minecraft EULA 已同意"
                } else {
                    "安装/启动前会要求同意 Minecraft EULA"
                })
                .size(11)
                .color(s.text_faint),
                button(text("查看 EULA").size(11))
                    .padding(4.0)
                    .style(style::secondary)
                    .on_press(Message::OpenEula),
                Space::new().width(Length::Fill),
                button(text("打开服务端目录").size(11))
                    .padding(4.0)
                    .style(style::secondary)
                    .on_press(Message::OpenServerFolder),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            row![install_btn, start_btn, join_btn, join_now_btn, stop_btn].spacing(8),
            text(format!("服务端目录: {server_dir}"))
                .size(10)
                .color(s.text_faint),
            text("说明：Fabric 版本会使用 Fabric Server（加载 mods/）；原版版本使用原版服务端。")
                .size(10)
                .color(s.text_faint),
            if app.config.server_online_mode {
                text("正版验证已开启：仅正版账号可加入。")
                    .size(10)
                    .color(s.text_faint)
            } else {
                text("正版验证关闭（online-mode=false）：离线账号可从本机加入，请勿通过端口转发暴露服务。")
                    .size(10)
                    .color(s.danger)
            },
        ]
        .spacing(10)
        .into(),
    );

    let log_lines: Vec<Element<Message>> = app
        .server_logs
        .iter()
        .rev()
        .take(800)
        .map(|line| text(line.clone()).size(10).color(s.text_muted).into())
        .collect();
    let log_body: Element<Message> = if log_lines.is_empty() {
        text("尚无服务端日志。安装并启动后可在这里查看（最新在上）。")
            .size(12)
            .color(s.text_faint)
            .into()
    } else {
        scrollable(column(log_lines).spacing(1))
            .height(Length::Fill)
            .width(Length::Fill)
            .into()
    };
    let log_card = container(
        column![
            text("服务端日志（最新在上）")
                .size(14)
                .color(s.text_primary),
            log_body,
        ]
        .spacing(8),
    )
    .padding(14)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(style::card);

    column![
        text("服务器").size(28).color(s.text_primary),
        text(app.status.as_str()).size(12).color(s.text_muted),
        rule::horizontal(1.0).style(style::rule_style),
        enter(t, 0.0, controls),
        enter(t, 0.12, log_card.into()),
    ]
    .spacing(12)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

// ── 设置页 ───────────────────────────────────────────────

fn advanced_page(app: &App, t: f32) -> Element<'_, Message> {
    let s = style::scheme_of(app.dark_mode);
    let graphics_picker = pick_list(
        vec![
            "default".to_string(),
            "opengl".to_string(),
            "vulkan".to_string(),
        ],
        Some(app.config.graphics_backend.clone()),
        Message::GraphicsBackendSelected,
    )
    .style(style::pick);

    let restrictions = section_card(
        s,
        "游戏限制",
        column![
            checkbox(app.config.disable_multiplayer)
                .label("禁用多人游戏（--disableMultiplayer）")
                .on_toggle(Message::ToggleDisableMultiplayer)
                .size(13)
                .style(style::checkbox_style),
            checkbox(app.config.disable_chat)
                .label("禁用聊天（--disableChat）")
                .on_toggle(Message::ToggleDisableChat)
                .size(13)
                .style(style::checkbox_style),
            text("这两个开关适合儿童/展示机；单人游戏不受影响。")
                .size(11)
                .color(s.text_faint),
        ]
        .spacing(10)
        .into(),
    );

    let window_graphics = section_card(
        s,
        "窗口与图形",
        column![
            checkbox(app.config.fullscreen)
                .label("全屏启动（--fullscreen）")
                .on_toggle(Message::ToggleFullscreen)
                .size(13)
                .style(style::checkbox_style),
            row![
                text("窗口宽").size(12).color(s.text_muted),
                text_input("1280", &app.window_width_input)
                    .on_input(Message::WindowWidthChanged)
                    .width(Length::from(90))
                    .style(style::text_field),
                text("高").size(12).color(s.text_muted),
                text_input("720", &app.window_height_input)
                    .on_input(Message::WindowHeightChanged)
                    .width(Length::from(90))
                    .style(style::text_field),
                text("图形后端").size(12).color(s.text_muted),
                graphics_picker,
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            text("窗口尺寸会以 --width/--height 传入；图形后端对应 26.3 的 --graphicsBackend（default/opengl/vulkan）。")
                .size(11)
                .color(s.text_faint),
            text("全屏时窗口尺寸由游戏/显示器决定；Vulkan 不可用时客户端可能回退或报错。")
                .size(11)
                .color(s.text_faint),
        ]
        .spacing(10)
        .into(),
    );

    column![
        text("高级启动").size(28).color(s.text_primary),
        text("这些参数在 26.3 的 Main 参数解析中已确认；旧版本可能忽略不支持的项。")
            .size(11)
            .color(s.text_faint),
        rule::horizontal(1.0).style(style::rule_style),
        enter(t, 0.0, restrictions),
        enter(t, 0.1, window_graphics),
        text(app.status.as_str()).size(12).color(s.text_muted),
    ]
    .spacing(14)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

// ── 设置页 ───────────────────────────────────────────────

fn settings_page(app: &App, t: f32) -> Element<'_, Message> {
    let s = style::scheme_of(app.dark_mode);
    let config_path = launcher::config::config_path()
        .map(|p| p.join("config.json").display().to_string())
        .unwrap_or_else(|e| e);
    let game_dir = launcher::config::resolve_game_dir(&app.config)
        .display()
        .to_string();
    let saves_dir = launcher::maps::root(&app.config).display().to_string();
    let versions_dir = if app.config.version_isolation {
        format!("{}/versions", app.config.root_dir)
    } else {
        format!("{}/game/versions", app.config.root_dir)
    };

    const CONCURRENCY: [usize; 6] = [4, 8, 16, 32, 64, 128];
    let concurrency_picker = pick_list(
        CONCURRENCY.to_vec(),
        CONCURRENCY
            .iter()
            .find(|&&c| c == app.config.download_concurrency),
        Message::ConcurrencySelected,
    )
    .placeholder("并发数")
    .style(style::pick);

    let reset_btn = if app.confirm_reset {
        button(text("确认重置？（再点一次执行）"))
            .padding(8.0)
            .style(style::danger)
            .on_press(Message::ResetAllPressed)
    } else {
        button(text("重置所有配置"))
            .padding(8.0)
            .style(style::danger)
            .on_press(Message::ResetAllPressed)
    };

    let appearance_card = section_card(
        s,
        "外观",
        column![
            checkbox(app.dark_mode)
                .label("暗色模式（牛皮纸黄 × 石板灰）")
                .on_toggle(Message::ToggleDarkMode)
                .size(13)
                .style(style::checkbox_style),
            checkbox(app.config.log_window)
                .label("启动游戏时打开独立日志窗口（stdout/stderr，超出 2000 行自动压缩）")
                .on_toggle(Message::ToggleLogWindow)
                .size(13)
                .style(style::checkbox_style),
            checkbox(app.config.brand_title)
                .label("标题界面左下角显示启动器名称（vanilla 生效；Fabric 暂不生效）")
                .on_toggle(Message::ToggleBrandTitle)
                .size(13)
                .style(style::checkbox_style),
            text_input("启动器名称，如 Deepslate", &app.config.brand_text)
                .on_input(Message::BrandTextChanged)
                .style(style::text_field),
            checkbox(app.config.apply_game_defaults)
                .label("首次启动时补齐语言与辅助功能默认值（普通实例只补缺失项）")
                .on_toggle(Message::ToggleGameDefaults)
                .size(13)
                .style(style::checkbox_style),
        ]
        .spacing(10)
        .into(),
    );
    // 常规：根目录 + 下载并发数合并为一张卡
    let general_card = section_card(
        s,
        "常规",
        column![
            checkbox(app.config.enhanced_experience)
                .label("增强体验：独立 Fabric 运行环境（26.3+）")
                .on_toggle(Message::ToggleEnhancedExperience)
                .size(13)
                .style(style::checkbox_style),
            text("最低要求 Minecraft 26.3；当前已接入 26.3。关闭后仍可选择并下载其他正式版。下次会话生效；首次进入世界时自动安装 Fabric 与 Bridge。")
                .size(11)
                .color(s.text_faint),
            text_input(
                "根目录，如 /home/xxx/Games/ObsidianLauncher",
                &app.config.root_dir
            )
            .on_input(Message::RootDirChanged)
            .style(style::text_field),
            row![
                text("下载并发数").size(13).color(s.text_muted),
                concurrency_picker,
            ]
            .spacing(10)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(10)
        .into(),
    );
    let enhanced_client_card: Element<Message> = if app.config.enhanced_experience {
        let language_choices = vec![
            "简体中文".to_string(),
            "繁體中文".to_string(),
            "English (US)".to_string(),
            "日本語".to_string(),
            "한국어".to_string(),
            "Français".to_string(),
            "Deutsch".to_string(),
            "Español".to_string(),
        ];
        let selected_language = match app.config.enhanced_language.as_str() {
            "zh_tw" => "繁體中文",
            "en_us" => "English (US)",
            "ja_jp" => "日本語",
            "ko_kr" => "한국어",
            "fr_fr" => "Français",
            "de_de" => "Deutsch",
            "es_es" => "Español",
            _ => "简体中文",
        }
        .to_string();
        let language_picker = pick_list(
            language_choices,
            Some(selected_language),
            |label: String| {
                let code = match label.as_str() {
                    "繁體中文" => "zh_tw",
                    "English (US)" => "en_us",
                    "日本語" => "ja_jp",
                    "한국어" => "ko_kr",
                    "Français" => "fr_fr",
                    "Deutsch" => "de_de",
                    "Español" => "es_es",
                    _ => "zh_cn",
                };
                Message::EnhancedLanguageSelected(code.to_string())
            },
        )
        .style(style::pick);
        let gc_choices = vec!["自动（JDK 默认）".to_string(), "低延迟（ZGC）".to_string()];
        let selected_gc = if app.config.enhanced_gc == "zgc" {
            "低延迟（ZGC）"
        } else {
            "自动（JDK 默认）"
        }
        .to_string();
        let gc_picker = pick_list(gc_choices, Some(selected_gc), |label: String| {
            Message::EnhancedGcSelected(if label == "低延迟（ZGC）" {
                "zgc".to_string()
            } else {
                "default".to_string()
            })
        })
        .style(style::pick);
        let memory_choices = vec![
            "自动".to_string(),
            "2 GB".to_string(),
            "4 GB".to_string(),
            "6 GB".to_string(),
            "8 GB".to_string(),
            "12 GB".to_string(),
            "16 GB".to_string(),
            "24 GB".to_string(),
            "32 GB".to_string(),
        ];
        let selected_memory = match app.config.enhanced_max_memory_mb {
            2048 => "2 GB",
            4096 => "4 GB",
            6144 => "6 GB",
            8192 => "8 GB",
            12288 => "12 GB",
            16384 => "16 GB",
            24576 => "24 GB",
            32768 => "32 GB",
            _ => "自动",
        }
        .to_string();
        let memory_picker = pick_list(memory_choices, Some(selected_memory), |label: String| {
            Message::EnhancedMemorySelected(match label.as_str() {
                "2 GB" => 2048,
                "4 GB" => 4096,
                "6 GB" => 6144,
                "8 GB" => 8192,
                "12 GB" => 12288,
                "16 GB" => 16384,
                "24 GB" => 24576,
                "32 GB" => 32768,
                _ => 0,
            })
        })
        .style(style::pick);
        section_card(
            s,
            "增强客户端",
            column![
                row![text("游戏语言").size(12).color(s.text_muted), language_picker]
                    .spacing(10)
                    .align_y(iced::Alignment::Center),
                row![text("垃圾回收").size(12).color(s.text_muted), gc_picker]
                    .spacing(10)
                    .align_y(iced::Alignment::Center),
                row![text("最大内存").size(12).color(s.text_muted), memory_picker]
                    .spacing(10)
                    .align_y(iced::Alignment::Center),
                checkbox(app.config.enhanced_compact_object_headers)
                    .label("紧凑对象头（Java 25+）")
                    .on_toggle(Message::EnhancedCompactHeaders)
                    .size(12)
                    .style(style::checkbox_style),
                text("仅作用于增强实例，保存后下次启动生效。默认保留 JDK 自动 GC 和内存策略；ZGC 优先降低停顿，可能增加 CPU 使用。")
                    .size(10)
                    .color(s.text_faint),
            ]
            .spacing(9)
            .into(),
        )
        .into()
    } else {
        Space::new().height(0).into()
    };
    let maps_card = section_card(
        s,
        "地图库",
        column![
            text(format!("世界存储: {}", saves_dir))
                .size(12)
                .color(s.text_muted),
            text("游戏实例的 saves/ 链接到这里；升级或重装游戏不会移动地图。")
                .size(11)
                .color(s.text_faint),
            text("旧版的版本隔离/存档隔离已停用；旧存档会在首次打开世界页时迁移并保留原目录。")
                .size(11)
                .color(s.text_faint),
        ]
        .spacing(10)
        .into(),
    );
    let dirs_card = section_card(
        s,
        "实际目录",
        column![
            text(format!("配置文件: {}", config_path))
                .size(12)
                .color(s.text_muted),
            text(format!("游戏目录: {}", game_dir))
                .size(12)
                .color(s.text_muted),
            text(format!("地图库: {}", saves_dir))
                .size(12)
                .color(s.text_muted),
            text(format!("版本目录: {}", versions_dir))
                .size(12)
                .color(s.text_muted),
        ]
        .spacing(5)
        .into(),
    );
    // 2×2 网格
    let grid_left = column![
        enter(t, 0.00, general_card),
        enter(t, 0.10, appearance_card),
        enter(t, 0.20, enhanced_client_card)
    ]
    .spacing(16)
    .width(Length::Fill);
    let grid_right = column![enter(t, 0.20, maps_card), enter(t, 0.30, dirs_card)]
        .spacing(16)
        .width(Length::Fill);

    column![
        text("设置").size(28).color(s.text_primary),
        rule::horizontal(1.0).style(style::rule_style),
        row![grid_left, grid_right].spacing(16),
        row![
            button(text("保存设置"))
                .padding(8.0)
                .style(style::primary)
                .on_press(Message::SaveSettings),
            reset_btn,
        ]
        .spacing(12),
        text("保存时会同时记录当前选中的档案与 Java 路径。重置会删除 config.json 与 profiles.json（游戏文件不受影响）。")
            .size(12)
            .color(s.text_faint),
    ]
    .spacing(16)
    .width(Length::Fill)
    .into()
}

// ── 入口 ─────────────────────────────────────────────────
