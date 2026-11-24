use crate::os_icons::{paint_mini_os_icon, paint_os_window_icon, DeviceIconStyle, OsIconAssets};
use crate::tray::{AppTray, TrayAction};
use eframe::egui::{
    self, Align2, Color32, ColorImage, CursorIcon, FontData, FontDefinitions, FontFamily, FontId, Id,
    LayerId, Order, Pos2, Rect, RichText, Rounding, Sense, Stroke, TextureHandle, Vec2,
};
use mouseroaming_core::{
    AppConfig, AppRole, ConnectionTransportMode, ConnectedPeerInfo, EngineCommand, EngineHandle,
    EngineSnapshot,
};
use mouseroaming_net::format_short_fingerprint;
use mouseroaming_proto::{Edge, OsKind};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Layout,
    Discovery,
    Settings,
    Diagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscoveryMode {
    Lan,
    Bluetooth,
}

#[derive(Debug, Clone)]
struct DragLayoutState {
    peer_id: String,
    origin_edge: Option<Edge>,
    grab_offset: Vec2,
    pointer_pos: Pos2,
    start_pos: Pos2,
    has_moved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DropTarget {
    Edge(Edge),
    Unbind,
}

pub struct MouseRoamingGui {
    engine: EngineHandle,
    tray: Option<AppTray>,
    app_logo: TextureHandle,
    os_icons: OsIconAssets,
    active_tab: Tab,
    discovery_mode: DiscoveryMode,
    draft_config: AppConfig,
    manual_connect_addr: String,
    manual_bluetooth_addr: String,
    window_visible: bool,
    status_banner: Option<String>,
    drag_layout: Option<DragLayoutState>,
}

impl MouseRoamingGui {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        engine: EngineHandle,
        tray: Option<AppTray>,
    ) -> Self {
        configure_fonts(&cc.egui_ctx);
        configure_style(&cc.egui_ctx);

        let logo_data =
            eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png"))
                .expect("Valid assets/icon.png");
        let logo_image = ColorImage::from_rgba_unmultiplied(
            [logo_data.width as usize, logo_data.height as usize],
            &logo_data.rgba,
        );
        let app_logo =
            cc.egui_ctx
                .load_texture("app_logo_header", logo_image, egui::TextureOptions::LINEAR);

        let os_icons = OsIconAssets::load(&cc.egui_ctx);
        let draft_config = engine.config();
        let manual_connect_addr = draft_config.connect_address.clone();
        let manual_bluetooth_addr = draft_config.bluetooth_target_addr.clone();

        Self {
            engine,
            tray,
            app_logo,
            os_icons,
            active_tab: Tab::Layout,
            discovery_mode: DiscoveryMode::Lan,
            draft_config,
            manual_connect_addr,
            manual_bluetooth_addr,
            window_visible: true,
            status_banner: None,
            drag_layout: None,
        }
    }

    fn handle_tray_events(&mut self, ctx: &egui::Context, snap: &EngineSnapshot) {
        let Some(ref tray) = self.tray else {
            return;
        };
        for action in tray.poll_actions() {
            match action {
                TrayAction::ToggleWindow => {
                    self.window_visible = !self.window_visible;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(self.window_visible));
                    if self.window_visible {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                }
                TrayAction::TogglePause => {
                    self.engine.send(EngineCommand::SetPaused(!snap.paused));
                }
                TrayAction::ReturnLocal => {
                    self.engine.send(EngineCommand::ManualSwitch(None));
                }
                TrayAction::Quit => {
                    self.engine.shutdown();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }
}

impl eframe::App for MouseRoamingGui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let snap = self.engine.snapshot();
        self.handle_tray_events(ctx, &snap);

        // When window is minimized, hide window so it stays resident in Windows System Tray / macOS Menu Bar
        if ctx.input(|i| i.viewport().minimized == Some(true)) {
            self.window_visible = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        // When window close is requested ([X]), cancel exit and hide to background tray / menu bar
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.window_visible = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        // Alert user and wake window when there is a pending connection request
        if !snap.pending_connections.is_empty() {
            if !self.window_visible {
                self.window_visible = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Critical,
            ));
            ctx.request_repaint_after(Duration::from_millis(300));
        } else if !self.window_visible {
            // Background resident mode: gently poll tray and connection requests
            ctx.request_repaint_after(Duration::from_millis(150));
            return;
        } else if self.drag_layout.is_some() {
            // Smooth 60+ fps updates while dragging; gentle polling otherwise
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(120));
        }

        // Top header bar
        egui::TopBottomPanel::top("header_bar").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.image((self.app_logo.id(), Vec2::splat(22.0)));
                ui.heading(
                    RichText::new("MouseRoaming")
                        .strong()
                        .color(Color32::from_rgb(96, 195, 255)),
                );
                ui.label(
                    RichText::new(format!(
                        "v{}  •  {} ({})  •  {}",
                        env!("CARGO_PKG_VERSION"),
                        snap.local_hostname,
                        snap.local_os.display_name(),
                        snap.local_ip,
                    ))
                    .weak(),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("結束").clicked() {
                        self.engine.shutdown();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }

                    if ui.button("🗕 縮小常駐").clicked() {
                        self.window_visible = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    }

                    if snap.running {
                        let pause_text = if snap.paused {
                            "▶ 恢復切換"
                        } else {
                            "⏸ 暫停切換"
                        };
                        if ui.button(pause_text).clicked() {
                            self.engine.send(EngineCommand::SetPaused(!snap.paused));
                        }
                        if ui.button("⏹ 停止服務").clicked() {
                            self.engine.send(EngineCommand::Stop);
                        }
                    } else if ui
                        .add(
                            egui::Button::new(RichText::new("▶ 啟動服務").color(Color32::WHITE))
                                .fill(Color32::from_rgb(22, 130, 82)),
                        )
                        .clicked()
                    {
                        self.engine.send(EngineCommand::Start);
                    }
                });
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.active_tab, Tab::Layout, "🖥 總覽與螢幕排列");
                ui.selectable_value(
                    &mut self.active_tab,
                    Tab::Discovery,
                    format!("📡 區網探索 ({})", snap.discovered_peers.len()),
                );
                ui.selectable_value(&mut self.active_tab, Tab::Settings, "⚙ 設定");
                ui.selectable_value(&mut self.active_tab, Tab::Diagnostics, "📋 權限與日誌");
                if !snap.pending_connections.is_empty() {
                    ui.separator();
                    ui.label(
                        RichText::new(format!("🔔 {} 個待審核連線請求", snap.pending_connections.len()))
                            .strong()
                            .color(Color32::from_rgb(255, 215, 80)),
                    );
                }
            });
            ui.add_space(4.0);
        });

        // Bottom status bar
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (dot_color, state_text) = if !snap.running {
                    (Color32::GRAY, "已停止")
                } else if snap.paused {
                    (Color32::from_rgb(245, 180, 60), "已暫停")
                } else {
                    (Color32::from_rgb(60, 210, 120), "運作中")
                };
                ui.label(RichText::new("●").color(dot_color));
                ui.label(format!("狀態: {state_text}"));
                ui.separator();
                ui.label(format!("目前游標: {}", snap.active_screen_label));
                ui.separator();
                ui.label(format!("TLS 指紋: {}", snap.local_fingerprint_short));
                if let Some(ref addr) = snap.listen_addr {
                    ui.separator();
                    ui.label(format!("監聽: {addr}"));
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if !snap.permission.granted {
                egui::Frame::none()
                    .fill(Color32::from_rgb(120, 42, 32))
                    .rounding(6.0)
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!("⚠ {}", snap.permission.message))
                                .strong()
                                .color(Color32::WHITE),
                        );
                        if let Some(ref adv) = snap.permission.advisory {
                            ui.label(RichText::new(adv).color(Color32::from_rgb(255, 220, 210)));
                        }
                    });
                ui.add_space(8.0);
            }

            if let Some(banner) = self.status_banner.clone() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(banner).color(Color32::from_rgb(120, 220, 160)));
                    if ui.small_button("✕").clicked() {
                        self.status_banner = None;
                    }
                });
                ui.add_space(4.0);
            }

            self.render_pending_connections(ui, &snap);

            match self.active_tab {
                Tab::Layout => self.render_layout_tab(ui, &snap),
                Tab::Discovery => self.render_discovery_tab(ui, &snap),
                Tab::Settings => self.render_settings_tab(ui, &snap),
                Tab::Diagnostics => self.render_diagnostics_tab(ui, &snap),
            }
        });
    }
}

impl MouseRoamingGui {
    fn render_pending_connections(&mut self, ui: &mut egui::Ui, snap: &EngineSnapshot) {
        if snap.pending_connections.is_empty() {
            return;
        }

        for pending in &snap.pending_connections {
            let rem_secs = pending.remaining_secs();
            let card_frame = egui::Frame::none()
                .fill(Color32::from_rgb(34, 38, 22))
                .stroke(Stroke::new(2.0_f32, Color32::from_rgb(240, 185, 45)))
                .rounding(Rounding::same(8.0))
                .inner_margin(12.0);

            card_frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("🔔 連線請求通知 (需要允許)")
                            .strong()
                            .size(16.0)
                            .color(Color32::from_rgb(255, 215, 80)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let timer_color = if rem_secs <= 5 {
                            Color32::from_rgb(255, 90, 80)
                        } else {
                            Color32::from_rgb(230, 200, 100)
                        };
                        ui.label(
                            RichText::new(format!("⏳ 倒數 {rem_secs} 秒自動逾時"))
                                .color(timer_color)
                                .small(),
                        );
                    });
                });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
                    paint_mini_os_icon(ui.painter(), icon_rect, pending.os, &self.os_icons);

                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("裝置名稱:").strong());
                            ui.label(
                                RichText::new(&pending.hostname)
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                            ui.separator();
                            ui.label(RichText::new("系統:").strong());
                            ui.label(pending.os.display_name());
                            ui.separator();
                            ui.label(RichText::new("來源位址:").strong());
                            ui.label(RichText::new(&pending.addr).monospace());
                        });
                        ui.label(
                            RichText::new(
                                "此裝置正在嘗試連線至本機進行滑鼠與鍵盤共享。請確認是否允許該連線：",
                            )
                            .color(Color32::from_rgb(220, 220, 220)),
                        );
                    });
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("✔ 允許本次連線 (Allow Once)")
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(28, 140, 75))
                            .min_size(Vec2::new(140.0, 28.0)),
                        )
                        .clicked()
                    {
                        self.engine.send(EngineCommand::AllowConnection {
                            id: pending.id,
                            remember: false,
                        });
                        self.status_banner = Some(format!("已允許「{}」連線", pending.hostname));
                    }

                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("🛡 允許並永久信任此裝置 (Allow & Remember)")
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(30, 110, 195))
                            .min_size(Vec2::new(210.0, 28.0)),
                        )
                        .clicked()
                    {
                        self.draft_config.allowed_peers.insert(
                            pending.peer_id.clone(),
                            pending.hostname.clone(),
                        );
                        self.engine.send(EngineCommand::AllowConnection {
                            id: pending.id,
                            remember: true,
                        });
                        self.status_banner = Some(format!(
                            "已允許「{}」連線並加入永久信任清單",
                            pending.hostname
                        ));
                    }

                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("✖ 拒絕連線 (Deny)")
                                    .color(Color32::from_rgb(255, 180, 180)),
                            )
                            .fill(Color32::from_rgb(140, 35, 35))
                            .min_size(Vec2::new(100.0, 28.0)),
                        )
                        .clicked()
                    {
                        self.engine.send(EngineCommand::DenyConnection { id: pending.id });
                        self.status_banner =
                            Some(format!("已拒絕「{}」的連線請求", pending.hostname));
                    }
                });
            });
            ui.add_space(8.0);
        }
    }

    fn render_layout_tab(&mut self, ui: &mut egui::Ui, snap: &EngineSnapshot) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            // Top summary row
            ui.horizontal(|ui| {
                ui.label(RichText::new("目前角色:").strong());
                ui.label(snap.role.label());
                ui.separator();
                ui.label(format!(
                    "本機解析度: {}×{}",
                    snap.local_screen.width, snap.local_screen.height
                ));
                if snap.role == AppRole::Server {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("游標立即返回本機 (Ctrl+Alt+Esc)").clicked() {
                            self.engine.send(EngineCommand::ManualSwitch(None));
                        }
                    });
                }
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.heading("視覺化螢幕排列");
                let hint = match snap.role {
                    AppRole::Server => {
                        "拖曳裝置圖示至上下左右即可定位或互換位置（雙擊可切換游標、右鍵開啟選單）"
                    }
                    AppRole::Client => "受控端預覽模式（螢幕排列位置由主控端 Server 統一拖曳設定）",
                };
                ui.label(RichText::new(hint).small().weak());
            });
            ui.add_space(6.0);

            // Update drag pointer position before rendering so preview is 1-frame-accurate
            let pointer_latest = ui.ctx().pointer_latest_pos();
            let primary_down = ui.ctx().input(|i| i.pointer.primary_down());
            if let Some(ref mut drag) = self.drag_layout {
                if let Some(pos) = pointer_latest {
                    drag.pointer_pos = pos;
                    if pos.distance(drag.start_pos) > 4.0 {
                        drag.has_moved = true;
                    }
                }
                // Cancel drag if the peer disconnected mid-drag
                if !snap.connected_peers.iter().any(|p| p.peer_id == drag.peer_id) {
                    self.drag_layout = None;
                }
            }

            // Compute arrangement stage rect
            let canvas_w = ui.available_width().max(560.0);
            let canvas_h = 344.0;
            let (canvas_rect, _canvas_resp) =
                ui.allocate_exact_size(Vec2::new(canvas_w, canvas_h), Sense::hover());

            let center = canvas_rect.center();
            let gap_x = (canvas_w * 0.28).clamp(172.0, 224.0);
            let gap_y = 110.0;
            let icon_size = Vec2::new(92.0, 62.0);
            let node_size = Vec2::new(152.0, 98.0);

            // Predict the unbind dock area below canvas (used for drop resolution)
            let predicted_unbind_rect = Rect::from_min_size(
                Pos2::new(canvas_rect.min.x, canvas_rect.max.y + 10.0),
                Vec2::new(canvas_w, 160.0),
            );

            let active_drop_target = self.drag_layout.as_ref().and_then(|drag| {
                if !drag.has_moved {
                    return None;
                }
                let dragged_center = drag.pointer_pos + drag.grab_offset;
                resolve_drop_target(
                    dragged_center,
                    canvas_rect,
                    center,
                    gap_x,
                    gap_y,
                    predicted_unbind_rect,
                )
            });

            // Paint arrangement canvas background & subtle blueprint dot grid
            {
                let painter = ui.painter_at(canvas_rect);
                painter.rect_filled(canvas_rect, Rounding::same(10.0), Color32::from_rgb(17, 21, 29));
                painter.rect_stroke(
                    canvas_rect,
                    Rounding::same(10.0),
                    Stroke::new(1.0_f32, Color32::from_rgb(42, 52, 68)),
                );
                paint_dot_grid(&painter, canvas_rect, 24.0);
            }

            let is_server = snap.role == AppRole::Server;
            let is_dragging_any = self
                .drag_layout
                .as_ref()
                .map(|d| d.has_moved)
                .unwrap_or(false);

            if is_dragging_any {
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            }

            // 1. Paint subtle edge connectors between Local PC and bound peers
            {
                let painter = ui.painter_at(canvas_rect);
                if is_server {
                    for edge in Edge::all() {
                        if let Some(peer) = snap.connected_peers.iter().find(|p| p.edge == Some(edge))
                        {
                            let is_being_dragged = self
                                .drag_layout
                                .as_ref()
                                .map(|d| d.has_moved && d.peer_id == peer.peer_id)
                                .unwrap_or(false);
                            if !is_being_dragged {
                                let is_active = snap.active_screen_label.contains(&peer.hostname);
                                let slot_c = edge_slot_center(center, edge, gap_x, gap_y);
                                paint_edge_connector(
                                    &painter,
                                    center,
                                    slot_c,
                                    edge,
                                    icon_size,
                                    is_active,
                                    false,
                                );
                            }
                        }
                    }

                    // Dynamic magnetic tether while dragging over an edge target
                    if let Some(DropTarget::Edge(target_edge)) = active_drop_target {
                        let target_c = edge_slot_center(center, target_edge, gap_x, gap_y);
                        paint_edge_connector(
                            &painter,
                            center,
                            target_c,
                            target_edge,
                            icon_size,
                            false,
                            true,
                        );
                    }
                } else if snap.connected_peers.first().is_some() {
                    let server_c = edge_slot_center(center, Edge::Left, gap_x, gap_y);
                    paint_edge_connector(
                        &painter,
                        center,
                        server_c,
                        Edge::Left,
                        icon_size,
                        false,
                        false,
                    );
                }
            }

            // 2. Render Center Node (This PC — OS Window Icon + Hostname + Local IP)
            {
                let local_rect = Rect::from_center_size(center, node_size);
                let local_resp =
                    ui.interact(local_rect, Id::new("layout_local_pc"), Sense::click());
                let local_active = snap.active_screen_label.contains("Local")
                    || snap.active_screen_label.contains("受控中");

                let painter = ui.painter_at(canvas_rect);
                paint_device_node(
                    &painter,
                    center,
                    icon_size,
                    snap.local_os,
                    &snap.local_hostname,
                    &snap.local_ip,
                    Some("本機"),
                    &self.os_icons,
                    DeviceIconStyle {
                        is_local: true,
                        is_active: local_active,
                        is_hovered: local_resp.hovered() && !is_dragging_any,
                        is_dragging: false,
                        alpha: 1.0,
                    },
                );

                if local_resp.double_clicked() && is_server {
                    self.engine.send(EngineCommand::ManualSwitch(None));
                }

                local_resp.on_hover_ui(|ui| {
                    ui.label(
                        RichText::new(format!("{}（本機）", snap.local_hostname)).strong(),
                    );
                    ui.label(format!("作業系統: {}", snap.local_os.display_name()));
                    ui.label(format!("IP 位址: {}", snap.local_ip));
                    ui.label(format!(
                        "螢幕解析度: {}×{}",
                        snap.local_screen.width, snap.local_screen.height
                    ));
                    if is_server {
                        ui.label(
                            RichText::new("中心參考主機 • 拖曳其他裝置圖示至四周即可定位")
                                .small()
                                .weak(),
                        );
                    }
                });
            }

            // 3. Render 4 Edge Slots (Server mode) or Connected Server preview (Client mode)
            if is_server {
                let mut started_drag: Option<DragLayoutState> = None;

                for edge in Edge::all() {
                    let slot_center = edge_slot_center(center, edge, gap_x, gap_y);
                    let slot_rect = Rect::from_center_size(slot_center, node_size);
                    let icon_rect = node_icon_rect(slot_center, icon_size);
                    let bound_peer = snap
                        .connected_peers
                        .iter()
                        .find(|p| p.edge == Some(edge));
                    let is_drop_hovered = active_drop_target == Some(DropTarget::Edge(edge));

                    let dragged_peer_id = self
                        .drag_layout
                        .as_ref()
                        .filter(|d| d.has_moved)
                        .map(|d| d.peer_id.as_str());
                    let is_origin_being_dragged = bound_peer
                        .map(|p| Some(p.peer_id.as_str()) == dragged_peer_id)
                        .unwrap_or(false);

                    match bound_peer {
                        Some(peer) => {
                            let resp = ui.interact(
                                slot_rect,
                                Id::new(("layout_edge_peer", edge, &peer.peer_id)),
                                Sense::click_and_drag(),
                            );
                            if resp.hovered() && !is_dragging_any {
                                ui.ctx().set_cursor_icon(CursorIcon::Grab);
                            }

                            if resp.drag_started_by(egui::PointerButton::Primary) {
                                if let Some(ptr) = ui.ctx().pointer_interact_pos().or(pointer_latest)
                                {
                                    started_drag = Some(DragLayoutState {
                                        peer_id: peer.peer_id.clone(),
                                        origin_edge: Some(edge),
                                        grab_offset: slot_center - ptr,
                                        pointer_pos: ptr,
                                        start_pos: ptr,
                                        has_moved: false,
                                    });
                                }
                            }

                            if resp.double_clicked() {
                                self.engine.send(EngineCommand::ManualSwitch(Some(edge)));
                            }

                            let peer_id_ctx = peer.peer_id.clone();
                            let peer_name_ctx = peer.hostname.clone();
                            resp.context_menu(|ui| {
                                ui.label(RichText::new(&peer_name_ctx).strong());
                                ui.separator();
                                if ui.button("🎯 切換游標至此裝置").clicked() {
                                    self.engine.send(EngineCommand::ManualSwitch(Some(edge)));
                                    ui.close_menu();
                                }
                                ui.menu_button("↔ 移動位置至...", |ui| {
                                    for target in Edge::all() {
                                        if target != edge
                                            && ui.button(edge_zh_short(target)).clicked()
                                        {
                                            self.engine.send(EngineCommand::BindPeerToEdge {
                                                peer_id: peer_id_ctx.clone(),
                                                edge: target,
                                            });
                                            ui.close_menu();
                                        }
                                    }
                                });
                                ui.separator();
                                if ui.button("✖ 解除螢幕定位").clicked() {
                                    self.engine.send(EngineCommand::UnbindEdge(edge));
                                    ui.close_menu();
                                }
                            });

                            let is_active = snap.active_screen_label.contains(&peer.hostname);
                            let painter = ui.painter_at(canvas_rect);

                            // If another device is being dragged onto this occupied slot, highlight swap target
                            if is_drop_hovered && !is_origin_being_dragged {
                                let swap_label = if self
                                    .drag_layout
                                    .as_ref()
                                    .and_then(|d| d.origin_edge)
                                    .is_some()
                                {
                                    format!("放開以與 {} 互換", truncate_label(&peer.hostname, 12))
                                } else {
                                    format!("放開以取代{}", edge_zh_short(edge))
                                };
                                paint_drop_highlight_ring(&painter, icon_rect.expand(6.0), &swap_label);
                            }

                            paint_device_node(
                                &painter,
                                slot_center,
                                icon_size,
                                peer.os,
                                &peer.hostname,
                                &peer.ip_address(),
                                None,
                                &self.os_icons,
                                DeviceIconStyle {
                                    is_local: false,
                                    is_active: is_active && !is_origin_being_dragged,
                                    is_hovered: resp.hovered() && !is_dragging_any,
                                    is_dragging: false,
                                    alpha: if is_origin_being_dragged { 0.28 } else { 1.0 },
                                },
                            );

                            if !is_dragging_any {
                                let peer_ip = peer.ip_address();
                                resp.on_hover_ui(|ui| {
                                    ui.label(RichText::new(&peer.hostname).strong());
                                    ui.label(format!("作業系統: {}", peer.os.display_name()));
                                    ui.label(format!("IP 位址: {peer_ip}"));
                                    ui.label(format!(
                                        "螢幕解析度: {}×{}",
                                        peer.screen.width, peer.screen.height
                                    ));
                                    ui.label(format!("目前位置: {}", edge_zh_short(edge)));
                                    ui.label(
                                        RichText::new(
                                            "拖曳圖示可重新定位或互換位置 • 雙擊切換游標 • 右鍵選單",
                                        )
                                        .small()
                                        .weak(),
                                    );
                                });
                            }
                        }
                        None => {
                            let painter = ui.painter_at(canvas_rect);
                            paint_empty_drop_slot(
                                &painter,
                                icon_rect,
                                edge,
                                is_dragging_any,
                                is_drop_hovered,
                            );
                        }
                    }
                }

                if let Some(drag) = started_drag {
                    self.drag_layout = Some(drag);
                }
            } else {
                // Client mode: show connected Server if available
                if let Some(server_peer) = snap.connected_peers.first() {
                    let server_center = edge_slot_center(center, Edge::Left, gap_x, gap_y);
                    let server_rect = Rect::from_center_size(server_center, node_size);
                    let resp = ui.interact(
                        server_rect,
                        Id::new(("layout_client_server", &server_peer.peer_id)),
                        Sense::hover(),
                    );
                    let painter = ui.painter_at(canvas_rect);
                    paint_device_node(
                        &painter,
                        server_center,
                        icon_size,
                        server_peer.os,
                        &server_peer.hostname,
                        &server_peer.ip_address(),
                        Some("主控端"),
                        &self.os_icons,
                        DeviceIconStyle {
                            is_local: false,
                            is_active: false,
                            is_hovered: resp.hovered(),
                            is_dragging: false,
                            alpha: 1.0,
                        },
                    );
                    let ip = server_peer.ip_address();
                    resp.on_hover_ui(|ui| {
                        ui.label(RichText::new(format!("{}（主控端 Server）", server_peer.hostname)).strong());
                        ui.label(format!("作業系統: {}", server_peer.os.display_name()));
                        ui.label(format!("IP 位址: {ip}"));
                        ui.label(format!(
                            "螢幕解析度: {}×{}",
                            server_peer.screen.width, server_peer.screen.height
                        ));
                    });
                } else {
                    let painter = ui.painter_at(canvas_rect);
                    painter.text(
                        Pos2::new(center.x, canvas_rect.max.y - 26.0),
                        Align2::CENTER_CENTER,
                        "尚未連線至主控端 Server，請前往「區網探索」或「設定」進行連線",
                        FontId::proportional(12.5),
                        Color32::from_rgb(148, 163, 184),
                    );
                }
            }

            ui.add_space(12.0);

            // 4. Connected & Unassigned Devices Dock (also serves as drag source & unbind drop zone)
            let is_unbind_hovered = is_server
                && is_dragging_any
                && self
                    .drag_layout
                    .as_ref()
                    .and_then(|d| d.origin_edge)
                    .is_some()
                && active_drop_target == Some(DropTarget::Unbind);

            let dock_fill = if is_unbind_hovered {
                Color32::from_rgba_unmultiplied(185, 56, 56, 42)
            } else {
                Color32::from_rgb(22, 26, 34)
            };
            let dock_stroke = if is_unbind_hovered {
                Stroke::new(1.8_f32, Color32::from_rgb(248, 113, 113))
            } else {
                Stroke::new(1.0_f32, Color32::from_rgb(44, 52, 66))
            };

            let mut started_dock_drag: Option<DragLayoutState> = None;

            egui::Frame::none()
                .fill(dock_fill)
                .stroke(dock_stroke)
                .rounding(8.0)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.set_min_width(canvas_w - 20.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "已連線裝置 ({})",
                                snap.connected_peers.len()
                            ))
                            .strong(),
                        );
                        if is_unbind_hovered {
                            ui.label(
                                RichText::new("  ⬇ 放開此處以解除螢幕定位")
                                    .strong()
                                    .color(Color32::from_rgb(252, 165, 165)),
                            );
                        } else if is_server && !snap.connected_peers.is_empty() {
                            ui.label(
                                RichText::new("（可由下方拖曳裝置至上方畫布定位，或拖回此區解除定位）")
                                    .small()
                                    .weak(),
                            );
                        }
                    });

                    ui.add_space(6.0);

                    if snap.connected_peers.is_empty() {
                        ui.label(
                            RichText::new(match snap.role {
                                AppRole::Server => "尚無 Client 連線。請在另一台電腦（Windows 或 macOS）開啟 MouseRoaming，切換為 Client 並於「區網探索」中一鍵連線。",
                                AppRole::Client => "尚未連線至 Server。請前往「區網探索」選擇主控端，或在「設定」中輸入主控端 IP。",
                            })
                            .weak(),
                        );
                    } else {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                            for peer in &snap.connected_peers {
                                let chip_size = Vec2::new(226.0, 52.0);
                                let (chip_rect, chip_resp) = ui.allocate_exact_size(
                                    chip_size,
                                    if is_server {
                                        Sense::click_and_drag()
                                    } else {
                                        Sense::hover()
                                    },
                                );

                                if is_server && chip_resp.hovered() && !is_dragging_any {
                                    ui.ctx().set_cursor_icon(CursorIcon::Grab);
                                }

                                if is_server
                                    && chip_resp.drag_started_by(egui::PointerButton::Primary)
                                {
                                    if let Some(ptr) =
                                        ui.ctx().pointer_interact_pos().or(pointer_latest)
                                    {
                                        started_dock_drag = Some(DragLayoutState {
                                            peer_id: peer.peer_id.clone(),
                                            origin_edge: peer.edge,
                                            grab_offset: Vec2::ZERO,
                                            pointer_pos: ptr,
                                            start_pos: ptr,
                                            has_moved: false,
                                        });
                                    }
                                }

                                paint_peer_dock_card(
                                    ui.painter(),
                                    chip_rect,
                                    peer,
                                    &self.os_icons,
                                    chip_resp.hovered() && is_server && !is_dragging_any,
                                    is_server,
                                );

                                if is_server {
                                    let pid = peer.peer_id.clone();
                                    let current_edge = peer.edge;
                                    chip_resp.context_menu(|ui| {
                                        ui.label(RichText::new(&peer.hostname).strong());
                                        ui.separator();
                                        for target in Edge::all() {
                                            let prefix = if current_edge == Some(target) {
                                                "✓ "
                                            } else {
                                                "   "
                                            };
                                            if ui
                                                .button(format!(
                                                    "{prefix}定位至{}",
                                                    edge_zh_short(target)
                                                ))
                                                .clicked()
                                            {
                                                self.engine.send(EngineCommand::BindPeerToEdge {
                                                    peer_id: pid.clone(),
                                                    edge: target,
                                                });
                                                ui.close_menu();
                                            }
                                        }
                                        if let Some(e) = current_edge {
                                            ui.separator();
                                            if ui.button("✖ 解除螢幕定位").clicked() {
                                                self.engine.send(EngineCommand::UnbindEdge(e));
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                }
                            }
                        });
                    }
                });

            if let Some(drag) = started_dock_drag {
                self.drag_layout = Some(drag);
            }

            // 5. Paint floating dragged device overlay & handle drop completion
            if let Some(ref drag) = self.drag_layout {
                if drag.has_moved {
                    if let Some(peer) = snap
                        .connected_peers
                        .iter()
                        .find(|p| p.peer_id == drag.peer_id)
                    {
                        let dragged_center = drag.pointer_pos + drag.grab_offset;
                        let overlay_painter = ui.ctx().layer_painter(LayerId::new(
                            Order::Tooltip,
                            Id::new("layout_drag_overlay"),
                        ));
                        paint_device_node(
                            &overlay_painter,
                            dragged_center,
                            icon_size,
                            peer.os,
                            &peer.hostname,
                            &peer.ip_address(),
                            None,
                            &self.os_icons,
                            DeviceIconStyle {
                                is_local: false,
                                is_active: false,
                                is_hovered: false,
                                is_dragging: true,
                                alpha: 0.96,
                            },
                        );
                    }
                }
            }

            if !primary_down {
                if let Some(drag) = self.drag_layout.take() {
                    if drag.has_moved && is_server {
                        let peer_name = snap
                            .connected_peers
                            .iter()
                            .find(|p| p.peer_id == drag.peer_id)
                            .map(|p| p.hostname.clone())
                            .unwrap_or_else(|| drag.peer_id.clone());

                        match active_drop_target {
                            Some(DropTarget::Edge(target_edge)) => {
                                if drag.origin_edge != Some(target_edge) {
                                    let occupant = snap
                                        .connected_peers
                                        .iter()
                                        .find(|p| p.edge == Some(target_edge))
                                        .map(|p| p.hostname.clone());

                                    self.engine.send(EngineCommand::BindPeerToEdge {
                                        peer_id: drag.peer_id,
                                        edge: target_edge,
                                    });

                                    self.status_banner = Some(
                                        match (drag.origin_edge, occupant) {
                                            (Some(_), Some(other)) => format!(
                                                "已將 {peer_name} 與 {other} 互換螢幕位置"
                                            ),
                                            _ => format!(
                                                "已將 {peer_name} 定位於本機{}",
                                                edge_zh_short(target_edge)
                                            ),
                                        },
                                    );
                                }
                            }
                            Some(DropTarget::Unbind) => {
                                if let Some(orig) = drag.origin_edge {
                                    self.engine.send(EngineCommand::UnbindEdge(orig));
                                    self.status_banner =
                                        Some(format!("已解除 {peer_name} 的螢幕定位"));
                                }
                            }
                            None => {}
                        }
                    }
                }
            }
        });
    }

    fn render_discovery_tab(&mut self, ui: &mut egui::Ui, snap: &EngineSnapshot) {
        ui.heading("本機 TLS 1.3 識別資訊");
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(format!(
                "裝置名稱: {} ({})  •  區域網路 IP: {}",
                snap.local_hostname,
                snap.local_os.display_name(),
                snap.local_ip
            ));
            if let Some(ref pan_ip) = snap.bluetooth_adapter.pan_ip {
                ui.label(format!("藍牙 PAN IP: {pan_ip}"));
            }
            ui.label(
                RichText::new(format!("配對短指紋: {}", snap.local_fingerprint_short))
                    .strong()
                    .color(Color32::from_rgb(110, 210, 255)),
            );
            ui.label(
                RichText::new(format!("完整 SHA-256: {}", snap.local_fingerprint_full))
                    .small()
                    .weak(),
            );
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.discovery_mode,
                DiscoveryMode::Lan,
                "🌐 區域網路探索 (Wi-Fi / LAN)",
            );
            ui.selectable_value(
                &mut self.discovery_mode,
                DiscoveryMode::Bluetooth,
                "📶 藍牙探索與主機連線 (Bluetooth)",
            );
        });
        ui.separator();
        ui.add_space(4.0);

        match self.discovery_mode {
            DiscoveryMode::Lan => {
                ui.heading("手動連線至 Server (IP:Port)");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.manual_connect_addr);
                    if ui.button("🔗 以 Client 身分連線").clicked() {
                        let addr = self.manual_connect_addr.trim().to_string();
                        if !addr.is_empty() {
                            self.draft_config.role = AppRole::Client;
                            self.draft_config.transport_mode = ConnectionTransportMode::Lan;
                            self.draft_config.connect_address = addr.clone();
                            self.engine.send(EngineCommand::ConnectToPeer {
                                addr: addr.clone(),
                                expected_fingerprint: None,
                            });
                            self.status_banner =
                                Some(format!("正在以 LAN 模式連線至 {addr}"));
                        }
                    }
                });

                ui.add_space(12.0);
                ui.separator();
                ui.heading("區域網路自動探索 (mDNS: _mouseroaming._tcp.local.)");
                ui.add_space(4.0);

                if snap.discovered_peers.is_empty() {
                    ui.label(
                        RichText::new("正在掃描同一區域網路中開啟 Server 模式的 MouseRoaming 裝置...")
                            .weak(),
                    );
                } else {
                    for peer in &snap.discovered_peers {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (icon_rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                                paint_mini_os_icon(
                                    ui.painter(),
                                    icon_rect,
                                    peer.os,
                                    &self.os_icons,
                                );

                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "{}  [{}]",
                                            peer.hostname,
                                            peer.os.display_name()
                                        ))
                                        .strong(),
                                    );
                                    let addr_str = peer
                                        .preferred_addr()
                                        .unwrap_or_else(|| format!("port {}", peer.port));
                                    ui.label(
                                        RichText::new(format!(
                                            "位址: {addr_str}  •  TLS 指紋: {}",
                                            format_short_fingerprint(&peer.fingerprint)
                                        ))
                                        .small()
                                        .weak(),
                                    );
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if let Some(addr) = peer.preferred_addr() {
                                            if ui.button("🔗 一鍵連線 (信任指紋)").clicked() {
                                                self.draft_config.role = AppRole::Client;
                                                self.draft_config.transport_mode =
                                                    ConnectionTransportMode::Lan;
                                                self.draft_config.connect_address = addr.clone();
                                                self.manual_connect_addr = addr.clone();
                                                self.engine.send(EngineCommand::ConnectToPeer {
                                                    addr: addr.clone(),
                                                    expected_fingerprint: Some(
                                                        peer.fingerprint.clone(),
                                                    ),
                                                });
                                                self.status_banner = Some(format!(
                                                    "已信任指紋並連線至 {} ({addr})",
                                                    peer.hostname
                                                ));
                                            }
                                        }
                                    },
                                );
                            });
                        });
                        ui.add_space(4.0);
                    }
                }
            }
            DiscoveryMode::Bluetooth => {
                ui.heading("本機藍牙介面狀態");
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let status_color = if snap.bluetooth_adapter.available {
                            Color32::from_rgb(60, 200, 120)
                        } else {
                            Color32::from_rgb(230, 140, 40)
                        };
                        ui.label(
                            RichText::new(format!(
                                "藍牙介面: {} ({})",
                                snap.bluetooth_adapter.name, snap.bluetooth_adapter.status
                            ))
                            .color(status_color)
                            .strong(),
                        );
                    });
                    if let Some(ref mac) = snap.bluetooth_adapter.mac {
                        ui.label(RichText::new(format!("藍牙 MAC: {mac}")).small().weak());
                    }
                    if let Some(ref pan_ip) = snap.bluetooth_adapter.pan_ip {
                        ui.label(
                            RichText::new(format!(
                                "藍牙個人區域網路 (PAN) IP: {pan_ip}"
                            ))
                            .color(Color32::from_rgb(100, 200, 255)),
                        );
                    } else {
                        ui.label(
                            RichText::new(
                                "未偵測到藍牙 PAN IP（若兩台電腦已藍牙配對，可開啟 Windows/macOS 藍牙網路共用建立直連）",
                            )
                            .small()
                            .weak(),
                        );
                    }

                    ui.horizontal(|ui| {
                        if ui.button("🔄 重新掃描藍牙").clicked() {
                            self.engine.send(EngineCommand::RefreshBluetooth);
                            self.status_banner = Some("已重新整理藍牙設備清單".into());
                        }
                        if ui.button("⚙ 開啟系統藍牙設定").clicked() {
                            self.engine
                                .send(EngineCommand::OpenSystemBluetoothSettings);
                        }
                    });
                });

                ui.add_space(8.0);
                ui.heading("手動藍牙主機連線 (Bluetooth IP / Host)");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.manual_bluetooth_addr);
                    if ui.button("🔗 透過藍牙連線").clicked() {
                        let target = self.manual_bluetooth_addr.trim().to_string();
                        if !target.is_empty() {
                            let target_addr = if target.contains(':') {
                                target.clone()
                            } else {
                                format!("{}:{}", target, self.draft_config.listen_port)
                            };
                            self.draft_config.role = AppRole::Client;
                            self.draft_config.transport_mode =
                                ConnectionTransportMode::Bluetooth;
                            self.draft_config.bluetooth_target_addr = target_addr.clone();
                            self.engine.send(EngineCommand::ConnectViaBluetooth {
                                addr: target_addr.clone(),
                                expected_fingerprint: None,
                            });
                            self.status_banner =
                                Some(format!("正在透過藍牙連線至 {target_addr}"));
                        }
                    }
                });

                ui.add_space(10.0);
                ui.heading("已配對之電腦主機清單");
                if snap.paired_bluetooth_hosts.is_empty() {
                    ui.label(
                        RichText::new(
                            "尚未在系統中找到已配對的電腦主機。請在作業系統設定中將兩台電腦進行藍牙配對並開啟網路共用。",
                        )
                        .weak(),
                    );
                } else {
                    for dev in &snap.paired_bluetooth_hosts {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(&dev.name).strong());
                                    let ip_text = dev
                                        .last_seen_ip
                                        .as_deref()
                                        .unwrap_or("未分配 PAN IP (請開啟藍牙個人區域網路共用)");
                                    ui.label(
                                        RichText::new(format!(
                                            "MAC: {}  •  {}",
                                            dev.mac, ip_text
                                        ))
                                        .small()
                                        .weak(),
                                    );
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if let Some(ref ip) = dev.last_seen_ip {
                                            let target_addr = format!(
                                                "{}:{}",
                                                ip, self.draft_config.listen_port
                                            );
                                            if ui.button("🔗 一鍵藍牙連線").clicked() {
                                                self.draft_config.role = AppRole::Client;
                                                self.draft_config.transport_mode =
                                                    ConnectionTransportMode::Bluetooth;
                                                self.draft_config.bluetooth_target_addr =
                                                    target_addr.clone();
                                                self.manual_bluetooth_addr =
                                                    target_addr.clone();
                                                self.engine.send(
                                                    EngineCommand::ConnectViaBluetooth {
                                                        addr: target_addr.clone(),
                                                        expected_fingerprint: None,
                                                    },
                                                );
                                                self.status_banner = Some(format!(
                                                    "正在連線至藍牙主機 {} ({target_addr})",
                                                    dev.name
                                                ));
                                            }
                                        } else {
                                            if ui.small_button("設定連線此主機").clicked() {
                                                self.manual_bluetooth_addr = format!(
                                                    "169.254.238.49:{}",
                                                    self.draft_config.listen_port
                                                );
                                            }
                                        }
                                    },
                                );
                            });
                        });
                        ui.add_space(4.0);
                    }
                }
            }
        }
    }

    fn render_settings_tab(&mut self, ui: &mut egui::Ui, _snap: &EngineSnapshot) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("連線角色與網路設定");
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("運作角色:");
                ui.radio_value(
                    &mut self.draft_config.role,
                    AppRole::Server,
                    AppRole::Server.label(),
                );
                ui.radio_value(
                    &mut self.draft_config.role,
                    AppRole::Client,
                    AppRole::Client.label(),
                );
            });

            ui.horizontal(|ui| {
                ui.label("連線傳輸媒介:");
                ui.radio_value(
                    &mut self.draft_config.transport_mode,
                    ConnectionTransportMode::Lan,
                    ConnectionTransportMode::Lan.label(),
                );
                ui.radio_value(
                    &mut self.draft_config.transport_mode,
                    ConnectionTransportMode::Bluetooth,
                    ConnectionTransportMode::Bluetooth.label(),
                );
                ui.radio_value(
                    &mut self.draft_config.transport_mode,
                    ConnectionTransportMode::Auto,
                    ConnectionTransportMode::Auto.label(),
                );
            });

            egui::Grid::new("settings_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("裝置顯示名稱:");
                    ui.text_edit_singleline(&mut self.draft_config.hostname);
                    ui.end_row();

                    ui.label("Server 監聽埠 (TCP/TLS):");
                    ui.add(
                        egui::DragValue::new(&mut self.draft_config.listen_port)
                            .range(1024..=65535),
                    );
                    ui.end_row();

                    ui.label("Client 連線目標 (LAN IP:Port):");
                    ui.text_edit_singleline(&mut self.draft_config.connect_address);
                    ui.end_row();

                    ui.label("藍牙連線目標 (BT IP:Port):");
                    ui.text_edit_singleline(&mut self.draft_config.bluetooth_target_addr);
                    ui.end_row();

                    ui.label("選填配對碼 (Pairing Token):");
                    ui.text_edit_singleline(&mut self.draft_config.pairing_token);
                    ui.end_row();

                    ui.label("新裝置預設邊緣位置:");
                    ui.horizontal(|ui| {
                        for e in Edge::all() {
                            ui.selectable_value(
                                &mut self.draft_config.default_client_edge,
                                e,
                                e.label(),
                            );
                        }
                    });
                    ui.end_row();
                });

            ui.add_space(10.0);
            ui.separator();
            ui.heading("跨平台操作手感與同步");
            ui.add_space(4.0);

            ui.checkbox(
                &mut self.draft_config.key_mapping.swap_ctrl_cmd_cross_os,
                "Windows ↔ macOS 自動互換 Ctrl 與 Command (⌘) 鍵（讓 Ctrl+C/V 直接對應 Cmd+C/V）",
            );
            ui.checkbox(
                &mut self.draft_config.key_mapping.invert_scroll_cross_os,
                "Windows ↔ macOS 跨機時自動反轉滾輪方向（抵銷 macOS 自然捲動差異）",
            );
            ui.checkbox(
                &mut self.draft_config.clipboard_sync_enabled,
                "啟用跨機剪貼簿文字自動同步（單次上限 2 MB）",
            );
            ui.checkbox(
                &mut self.draft_config.auto_start_service,
                "啟動應用程式時自動開始服務",
            );

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("螢幕角落防誤觸緩衝 (Dead-corner px):");
                ui.add(
                    egui::Slider::new(&mut self.draft_config.dead_corner_px, 0..=80).suffix(" px"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("邊緣切換冷卻時間 (防止來回彈跳):");
                ui.add(
                    egui::Slider::new(&mut self.draft_config.edge_switch_cooldown_ms, 0..=500)
                        .suffix(" ms"),
                );
            });

            ui.add_space(10.0);
            ui.separator();
            ui.heading("🔒 連線安全與通知設定");
            ui.add_space(4.0);

            ui.checkbox(
                &mut self.draft_config.require_connection_approval,
                "連線需要被允許 (連線審核：新裝置嘗試連線時彈出通知並等待本機核准)",
            );
            ui.checkbox(
                &mut self.draft_config.notify_on_connection,
                "啟用連線通知與提示音 (新連線請求、連線成功、斷線時發送桌面通知與提示音)",
            );

            ui.add_space(6.0);
            ui.label(RichText::new("已永久信任／允許連線的裝置清單:").strong());
            if self.draft_config.allowed_peers.is_empty() {
                ui.label(
                    RichText::new("（目前尚無已儲存的信任裝置。於收到連線請求時點選「允許並永久信任此裝置」即可加入清單）")
                        .small()
                        .weak(),
                );
            } else {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    let mut to_remove = Vec::new();
                    for (pid, name) in &self.draft_config.allowed_peers {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).strong());
                            ui.label(RichText::new(format!("(ID: {pid})")).small().weak());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("🗑 撤銷允許").clicked() {
                                    to_remove.push(pid.clone());
                                }
                            });
                        });
                    }
                    for pid in to_remove {
                        self.draft_config.allowed_peers.remove(&pid);
                        self.engine.send(EngineCommand::RevokeAllowedPeer(pid));
                        self.status_banner = Some("已撤銷該裝置的連線允許許可".into());
                    }
                    if !self.draft_config.allowed_peers.is_empty() {
                        ui.add_space(4.0);
                        if ui.button("全部清除信任裝置").clicked() {
                            for pid in self.draft_config.allowed_peers.keys().cloned().collect::<Vec<_>>() {
                                self.engine.send(EngineCommand::RevokeAllowedPeer(pid));
                            }
                            self.draft_config.allowed_peers.clear();
                            self.status_banner = Some("已清除所有已信任裝置".into());
                        }
                    }
                });
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("💾 儲存並套用設定").color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(35, 115, 200)),
                    )
                    .clicked()
                {
                    self.manual_connect_addr = self.draft_config.connect_address.clone();
                    self.manual_bluetooth_addr = self.draft_config.bluetooth_target_addr.clone();
                    self.engine
                        .send(EngineCommand::UpdateConfig(self.draft_config.clone()));
                    self.status_banner = Some("設定已儲存並即時套用！".into());
                }

                if ui.button("清除所有已信任指紋 (TOFU)").clicked() {
                    self.draft_config.trusted_fingerprints.clear();
                    self.engine
                        .send(EngineCommand::UpdateConfig(self.draft_config.clone()));
                    self.status_banner = Some("已清除所有已記錄的 TOFU 憑證指紋。".into());
                }
            });
        });
    }

    fn render_diagnostics_tab(&mut self, ui: &mut egui::Ui, snap: &EngineSnapshot) {
        ui.heading("系統權限與安全快捷鍵");
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(format!(
                "權限狀態: {} {}",
                if snap.permission.granted { "✅" } else { "⚠" },
                snap.permission.message
            ));
            if let Some(ref adv) = snap.permission.advisory {
                ui.label(RichText::new(adv).weak());
            }
            ui.separator();
            ui.label(
                "🆘 緊急逃生快捷鍵：Ctrl + Alt + Esc（任何情況下立即解除遠端鎖定、游標返回本機）",
            );
            ui.label("🛡 傳輸加密：TLS 1.3 + 自簽憑證 SHA-256 TOFU 指紋驗證");
        });

        ui.add_space(10.0);
        ui.heading("即時系統日誌");
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .max_height(260.0)
            .show(ui, |ui| {
                for line in &snap.logs {
                    ui.label(RichText::new(line).monospace().small());
                }
            });
    }
}

fn edge_zh_short(edge: Edge) -> &'static str {
    match edge {
        Edge::Left => "左側",
        Edge::Right => "右側",
        Edge::Top => "上方",
        Edge::Bottom => "下方",
    }
}

fn edge_directional_hint(edge: Edge) -> &'static str {
    match edge {
        Edge::Left => "← 左側",
        Edge::Right => "右側 →",
        Edge::Top => "↑ 上方",
        Edge::Bottom => "↓ 下方",
    }
}

fn edge_slot_center(center: Pos2, edge: Edge, gap_x: f32, gap_y: f32) -> Pos2 {
    match edge {
        Edge::Left => Pos2::new(center.x - gap_x, center.y),
        Edge::Right => Pos2::new(center.x + gap_x, center.y),
        Edge::Top => Pos2::new(center.x, center.y - gap_y),
        Edge::Bottom => Pos2::new(center.x, center.y + gap_y),
    }
}

fn node_icon_rect(node_center: Pos2, icon_size: Vec2) -> Rect {
    let icon_center = Pos2::new(node_center.x, node_center.y - 17.0);
    Rect::from_center_size(icon_center, icon_size)
}

fn resolve_drop_target(
    dragged_center: Pos2,
    canvas_rect: Rect,
    center: Pos2,
    gap_x: f32,
    gap_y: f32,
    unbind_rect: Rect,
) -> Option<DropTarget> {
    if unbind_rect.contains(dragged_center) {
        return Some(DropTarget::Unbind);
    }
    if canvas_rect.expand(28.0).contains(dragged_center) {
        let delta = dragged_center - center;
        if delta.length() < 26.0 {
            return None;
        }
        let nx = delta.x / gap_x.max(1.0);
        let ny = delta.y / gap_y.max(1.0);
        let edge = if nx.abs() >= ny.abs() {
            if nx < 0.0 {
                Edge::Left
            } else {
                Edge::Right
            }
        } else if ny < 0.0 {
            Edge::Top
        } else {
            Edge::Bottom
        };
        return Some(DropTarget::Edge(edge));
    }
    None
}

fn truncate_label(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        text.to_string()
    } else if max_chars <= 1 {
        "…".to_string()
    } else {
        let prefix: String = text.chars().take(max_chars - 1).collect();
        format!("{prefix}…")
    }
}

fn paint_dot_grid(painter: &egui::Painter, rect: Rect, step: f32) {
    let dot_color = Color32::from_rgba_unmultiplied(148, 163, 184, 24);
    let mut y = rect.min.y + step * 0.5;
    while y < rect.max.y {
        let mut x = rect.min.x + step * 0.5;
        while x < rect.max.x {
            painter.circle_filled(Pos2::new(x, y), 1.1, dot_color);
            x += step;
        }
        y += step;
    }
}

fn paint_dashed_rect(
    painter: &egui::Painter,
    rect: Rect,
    dash_len: f32,
    gap_len: f32,
    stroke: Stroke,
) {
    let edges = [
        (rect.left_top(), rect.right_top()),
        (rect.right_top(), rect.right_bottom()),
        (rect.right_bottom(), rect.left_bottom()),
        (rect.left_bottom(), rect.left_top()),
    ];
    let stride = (dash_len + gap_len).max(1.0);
    for (start, end) in edges {
        let diff = end - start;
        let len = diff.length();
        if len <= 0.1 {
            continue;
        }
        let dir = diff / len;
        let mut d = 0.0;
        while d < len {
            let seg_end = (d + dash_len).min(len);
            painter.line_segment([start + dir * d, start + dir * seg_end], stroke);
            d += stride;
        }
    }
}

fn paint_edge_connector(
    painter: &egui::Painter,
    center: Pos2,
    slot_center: Pos2,
    edge: Edge,
    icon_size: Vec2,
    is_active: bool,
    is_preview: bool,
) {
    let c_icon = node_icon_rect(center, icon_size);
    let s_icon = node_icon_rect(slot_center, icon_size);
    let (p0, p1) = match edge {
        Edge::Left => (
            Pos2::new(c_icon.min.x - 6.0, c_icon.center().y),
            Pos2::new(s_icon.max.x + 6.0, s_icon.center().y),
        ),
        Edge::Right => (
            Pos2::new(c_icon.max.x + 6.0, c_icon.center().y),
            Pos2::new(s_icon.min.x - 6.0, s_icon.center().y),
        ),
        Edge::Top => (
            Pos2::new(c_icon.center().x, c_icon.min.y - 6.0),
            Pos2::new(s_icon.center().x, s_icon.max.y + 36.0),
        ),
        Edge::Bottom => (
            Pos2::new(c_icon.center().x, c_icon.max.y + 36.0),
            Pos2::new(s_icon.center().x, s_icon.min.y - 6.0),
        ),
    };

    let color = if is_preview {
        Color32::from_rgb(56, 189, 248)
    } else if is_active {
        Color32::from_rgb(52, 211, 153)
    } else {
        Color32::from_rgba_unmultiplied(100, 148, 210, 150)
    };
    let width = if is_active || is_preview { 2.2_f32 } else { 1.5_f32 };

    if is_preview {
        let diff = p1 - p0;
        let len = diff.length();
        if len > 1.0 {
            let dir = diff / len;
            let mut d = 0.0;
            while d < len {
                let seg_end = (d + 6.0).min(len);
                painter.line_segment([p0 + dir * d, p0 + dir * seg_end], Stroke::new(width, color));
                d += 11.0;
            }
        }
    } else {
        painter.line_segment([p0, p1], Stroke::new(width, color));
        let mid = Pos2::new((p0.x + p1.x) * 0.5, (p0.y + p1.y) * 0.5);
        painter.circle_filled(mid, 3.0, color);
    }
}

fn paint_empty_drop_slot(
    painter: &egui::Painter,
    icon_rect: Rect,
    edge: Edge,
    is_dragging: bool,
    is_hovered: bool,
) {
    let fill = if is_hovered {
        Color32::from_rgba_unmultiplied(56, 189, 248, 38)
    } else if is_dragging {
        Color32::from_rgba_unmultiplied(56, 189, 248, 14)
    } else {
        Color32::from_rgba_unmultiplied(148, 163, 184, 7)
    };

    let border_color = if is_hovered {
        Color32::from_rgb(56, 189, 248)
    } else if is_dragging {
        Color32::from_rgba_unmultiplied(125, 211, 252, 160)
    } else {
        Color32::from_rgba_unmultiplied(100, 116, 139, 85)
    };

    painter.rect_filled(icon_rect, Rounding::same(8.0), fill);
    paint_dashed_rect(
        painter,
        icon_rect,
        6.0,
        4.0,
        Stroke::new(if is_hovered { 2.0_f32 } else { 1.2_f32 }, border_color),
    );

    let text = if is_hovered {
        format!("放開以定位於{}", edge_zh_short(edge))
    } else {
        edge_directional_hint(edge).to_string()
    };
    let text_color = if is_hovered {
        Color32::from_rgb(186, 230, 253)
    } else if is_dragging {
        Color32::from_rgb(148, 197, 232)
    } else {
        Color32::from_rgba_unmultiplied(148, 163, 184, 150)
    };

    painter.text(
        icon_rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(if is_hovered { 12.0 } else { 11.5 }),
        text_color,
    );
}

fn paint_drop_highlight_ring(painter: &egui::Painter, rect: Rect, label: &str) {
    painter.rect_filled(
        rect,
        Rounding::same(11.0),
        Color32::from_rgba_unmultiplied(56, 189, 248, 30),
    );
    paint_dashed_rect(
        painter,
        rect,
        6.0,
        4.0,
        Stroke::new(2.0_f32, Color32::from_rgb(56, 189, 248)),
    );
    let badge_center = Pos2::new(rect.center().x, rect.min.y - 10.0);
    let badge_rect = Rect::from_center_size(badge_center, Vec2::new(132.0, 18.0));
    painter.rect_filled(
        badge_rect,
        Rounding::same(9.0),
        Color32::from_rgb(14, 116, 144),
    );
    painter.text(
        badge_center,
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(10.5),
        Color32::WHITE,
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_device_node(
    painter: &egui::Painter,
    node_center: Pos2,
    icon_size: Vec2,
    os: OsKind,
    hostname: &str,
    ip: &str,
    badge: Option<&str>,
    assets: &OsIconAssets,
    style: DeviceIconStyle,
) {
    let icon_rect = node_icon_rect(node_center, icon_size);
    paint_os_window_icon(painter, icon_rect, os, assets, style);

    let alpha_u8 = (255.0 * style.alpha.clamp(0.0, 1.0)).round() as u8;

    // Optional small top-left/top-center role pill (e.g. "本機" or "主控端")
    if let Some(tag) = badge {
        let pill_rect = Rect::from_center_size(
            Pos2::new(icon_rect.center().x, icon_rect.min.y - 1.0),
            Vec2::new(42.0, 16.0),
        );
        let pill_fill = if style.is_active {
            Color32::from_rgba_unmultiplied(16, 140, 92, alpha_u8)
        } else {
            Color32::from_rgba_unmultiplied(37, 99, 235, alpha_u8)
        };
        painter.rect_filled(pill_rect, Rounding::same(8.0), pill_fill);
        painter.text(
            pill_rect.center(),
            Align2::CENTER_CENTER,
            tag,
            FontId::proportional(10.0),
            Color32::from_rgba_unmultiplied(255, 255, 255, alpha_u8),
        );
    }

    // Hostname directly below the OS icon
    let name_color = if style.is_active {
        Color32::from_rgba_unmultiplied(110, 231, 183, alpha_u8)
    } else {
        Color32::from_rgba_unmultiplied(241, 245, 249, alpha_u8)
    };
    let display_name = truncate_label(hostname, 18);
    painter.text(
        Pos2::new(node_center.x, icon_rect.max.y + 7.0),
        Align2::CENTER_TOP,
        display_name,
        FontId::proportional(13.0),
        name_color,
    );

    // IP address directly below the hostname
    let ip_color = Color32::from_rgba_unmultiplied(148, 197, 232, alpha_u8);
    painter.text(
        Pos2::new(node_center.x, icon_rect.max.y + 24.0),
        Align2::CENTER_TOP,
        ip,
        FontId::monospace(11.5),
        ip_color,
    );
}

fn paint_peer_dock_card(
    painter: &egui::Painter,
    rect: Rect,
    peer: &ConnectedPeerInfo,
    assets: &OsIconAssets,
    is_hovered: bool,
    is_server: bool,
) {
    let fill = if is_hovered {
        Color32::from_rgb(34, 44, 62)
    } else {
        Color32::from_rgb(28, 34, 46)
    };
    let stroke = if is_hovered {
        Stroke::new(1.5_f32, Color32::from_rgb(96, 165, 250))
    } else {
        Stroke::new(1.0_f32, Color32::from_rgb(54, 66, 86))
    };

    painter.rect_filled(rect, Rounding::same(8.0), fill);
    painter.rect_stroke(rect, Rounding::same(8.0), stroke);

    // Mini OS icon on the left
    let icon_rect = Rect::from_center_size(
        Pos2::new(rect.min.x + 22.0, rect.center().y),
        Vec2::splat(24.0),
    );
    paint_mini_os_icon(painter, icon_rect, peer.os, assets);

    // Hostname & IP
    let text_x = rect.min.x + 42.0;
    painter.text(
        Pos2::new(text_x, rect.min.y + 9.0),
        Align2::LEFT_TOP,
        truncate_label(&peer.hostname, 15),
        FontId::proportional(13.0),
        Color32::from_rgb(241, 245, 249),
    );
    painter.text(
        Pos2::new(text_x, rect.min.y + 28.0),
        Align2::LEFT_TOP,
        peer.ip_address(),
        FontId::monospace(11.0),
        Color32::from_rgb(148, 197, 232),
    );

    // Position badge on the right
    if is_server {
        let (badge_text, badge_bg, badge_fg) = match peer.edge {
            Some(edge) => (
                edge_zh_short(edge),
                Color32::from_rgb(24, 72, 112),
                Color32::from_rgb(186, 230, 253),
            ),
            None => (
                "未定位",
                Color32::from_rgb(76, 56, 28),
                Color32::from_rgb(253, 224, 71),
            ),
        };
        let pill = Rect::from_center_size(
            Pos2::new(rect.max.x - 28.0, rect.center().y),
            Vec2::new(42.0, 20.0),
        );
        painter.rect_filled(pill, Rounding::same(10.0), badge_bg);
        painter.text(
            pill.center(),
            Align2::CENTER_CENTER,
            badge_text,
            FontId::proportional(10.5),
            badge_fg,
        );
    }
}

fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    let candidates: &[&str] = if cfg!(target_os = "windows") {
        &[
            r"C:\Windows\Fonts\msjh.ttc",
            r"C:\Windows\Fonts\msyh.ttc",
            r"C:\Windows\Fonts\mingliu.ttc",
        ]
    } else if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/STHeiti Medium.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
        ]
    } else {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        ]
    };

    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("system_cjk".to_owned(), FontData::from_owned(bytes));
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .push("system_cjk".to_owned());
            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .push("system_cjk".to_owned());
            break;
        }
    }

    ctx.set_fonts(fonts);
}

fn configure_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_rounding = egui::Rounding::same(8.0);
    ctx.set_visuals(visuals);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_drop_target_snaps_to_all_edges_and_unbind() {
        let canvas = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(600.0, 344.0));
        let center = canvas.center(); // (300, 172)
        let unbind = Rect::from_min_size(Pos2::new(0.0, 354.0), Vec2::new(600.0, 140.0));
        let gap_x = 180.0;
        let gap_y = 110.0;

        // Deadzone over local center returns None
        assert_eq!(
            resolve_drop_target(center + Vec2::new(5.0, -4.0), canvas, center, gap_x, gap_y, unbind),
            None
        );

        // Dragging to each quadrant snaps magnetically to the right edge
        assert_eq!(
            resolve_drop_target(Pos2::new(110.0, 175.0), canvas, center, gap_x, gap_y, unbind),
            Some(DropTarget::Edge(Edge::Left))
        );
        assert_eq!(
            resolve_drop_target(Pos2::new(490.0, 170.0), canvas, center, gap_x, gap_y, unbind),
            Some(DropTarget::Edge(Edge::Right))
        );
        assert_eq!(
            resolve_drop_target(Pos2::new(305.0, 50.0), canvas, center, gap_x, gap_y, unbind),
            Some(DropTarget::Edge(Edge::Top))
        );
        assert_eq!(
            resolve_drop_target(Pos2::new(295.0, 290.0), canvas, center, gap_x, gap_y, unbind),
            Some(DropTarget::Edge(Edge::Bottom))
        );

        // Dragging down into dock triggers Unbind
        assert_eq!(
            resolve_drop_target(Pos2::new(250.0, 400.0), canvas, center, gap_x, gap_y, unbind),
            Some(DropTarget::Unbind)
        );
    }

    #[test]
    fn truncate_label_handles_unicode_cleanly() {
        assert_eq!(truncate_label("MacBook-Pro", 18), "MacBook-Pro");
        assert_eq!(truncate_label("我的測試電腦主機名稱很長", 6), "我的測試電…");
    }
}
