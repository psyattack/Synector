//! Standalone HSVA colour editor shared by the GUI views.

use egui::{pos2, vec2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, Ui};

use crate::gui::theme::{self, Tokens};

pub(crate) fn inline_color_editor(
    ui: &mut Ui,
    color: &mut [u8; 4],
    has_alpha: bool,
    t: &Tokens,
    loc: &crate::i18n::Strings,
) {
    egui::Frame::new()
        .fill(t.sunken)
        .stroke(Stroke::new(1.0, t.accent_ink))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;

            // Maintain stable HSVA state across frames to prevent hue/saturation snapping at extremes
            let hsva_id = ui.scope_id().with("custom_picker_hsva");
            let last_rgba_id = ui.scope_id().with("custom_picker_last_rgba");

            let last_rgba: [u8; 4] = ui.ctx().data(|d| d.get_temp(last_rgba_id)).unwrap_or([255, 255, 255, 255]);
            let mut hsva: egui::ecolor::Hsva = ui.ctx().data(|d| d.get_temp(hsva_id)).unwrap_or_else(|| {
                egui::ecolor::Hsva::from_srgba_unmultiplied(*color)
            });

            if *color != last_rgba {
                hsva = egui::ecolor::Hsva::from_srgba_unmultiplied(*color);
            }

            // Row 1: Quick Presets Palette + Direct HEX input in single row
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let presets: [(&str, [u8; 4]); 7] = [
                    (loc.overlay_studio_palette_cyan, [0, 212, 255, 255]),
                    (loc.overlay_studio_palette_green, [34, 197, 94, 255]),
                    (loc.overlay_studio_palette_red, [239, 68, 68, 255]),
                    (loc.overlay_studio_palette_amber, [245, 158, 11, 255]),
                    (loc.overlay_studio_palette_white, [255, 255, 255, 255]),
                    (loc.overlay_studio_palette_dark, [24, 28, 38, 220]),
                    (loc.overlay_studio_palette_clear, [0, 0, 0, 0]),
                ];
                for (name, col) in presets {
                    let (rect, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                    let c32 = Color32::from_rgba_unmultiplied(col[0], col[1], col[2], col[3]);
                    ui.painter().rect_filled(rect, CornerRadius::same(2), c32);
                    ui.painter().rect_stroke(rect, CornerRadius::same(2), t.hairline(), egui::StrokeKind::Inside);
                    if resp.on_hover_text(name).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        color[0] = col[0];
                        color[1] = col[1];
                        color[2] = col[2];
                        if has_alpha {
                            color[3] = col[3];
                        }
                        hsva = egui::ecolor::Hsva::from_srgba_unmultiplied(*color);
                    }
                }

                ui.add_space(2.0);

                let hex_id = ui.scope_id().with("hex_edit_buf");
                let mut hex_val: String = ui.ctx().data(|d| d.get_temp(hex_id)).unwrap_or_else(|| {
                    if has_alpha {
                        format!("#{:02X}{:02X}{:02X}{:02X}", color[0], color[1], color[2], color[3])
                    } else {
                        format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2])
                    }
                });
                let prev_hex = hex_val.clone();
                let txt_resp = ui.add(
                    egui::TextEdit::singleline(&mut hex_val)
                        .desired_width(ui.available_width())
                        .font(FontId::new(11.0, theme::mono()))
                );
                if txt_resp.changed() {
                    let clean = hex_val.trim().trim_start_matches('#');
                    if clean.len() == 6 {
                        if let Ok(v) = u32::from_str_radix(clean, 16) {
                            color[0] = ((v >> 16) & 0xFF) as u8;
                            color[1] = ((v >> 8) & 0xFF) as u8;
                            color[2] = (v & 0xFF) as u8;
                            hsva = egui::ecolor::Hsva::from_srgba_unmultiplied(*color);
                        }
                    } else if clean.len() == 8 {
                        if let Ok(v) = u32::from_str_radix(clean, 16) {
                            color[0] = ((v >> 24) & 0xFF) as u8;
                            color[1] = ((v >> 16) & 0xFF) as u8;
                            color[2] = ((v >> 8) & 0xFF) as u8;
                            color[3] = (v & 0xFF) as u8;
                            hsva = egui::ecolor::Hsva::from_srgba_unmultiplied(*color);
                        }
                    }
                    ui.ctx().data_mut(|d| d.insert_temp(hex_id, hex_val.clone()));
                } else if !txt_resp.has_focus() {
                    let latest_hex = if has_alpha {
                        format!("#{:02X}{:02X}{:02X}{:02X}", color[0], color[1], color[2], color[3])
                    } else {
                        format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2])
                    };
                    if latest_hex != prev_hex {
                        ui.ctx().data_mut(|d| d.insert_temp(hex_id, latest_hex));
                    }
                }
            });

            // Row 2: Full-width 2D Saturation / Value gradient canvas
            let full_w = ui.available_width();
            let sv_h = 110.0;
            let (sv_rect, sv_resp) = ui.allocate_exact_size(vec2(full_w, sv_h), Sense::click_and_drag());

            let pure_hue = Color32::from(egui::ecolor::Hsva::new(hsva.h, 1.0, 1.0, 1.0));
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(pos2(sv_rect.left(), sv_rect.top()), Color32::WHITE);
            mesh.colored_vertex(pos2(sv_rect.right(), sv_rect.top()), pure_hue);
            mesh.colored_vertex(pos2(sv_rect.left(), sv_rect.bottom()), Color32::BLACK);
            mesh.colored_vertex(pos2(sv_rect.right(), sv_rect.bottom()), Color32::BLACK);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(2, 1, 3);
            ui.painter().add(egui::Shape::mesh(mesh));
            ui.painter().rect_stroke(sv_rect, CornerRadius::same(3), t.hairline(), egui::StrokeKind::Inside);

            let pointer_pos = ui.input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()));
            if sv_resp.clicked() || sv_resp.dragged() {
                if let Some(pos) = pointer_pos {
                    hsva.s = ((pos.x - sv_rect.left()) / sv_rect.width()).clamp(0.0, 1.0);
                    hsva.v = (1.0 - (pos.y - sv_rect.top()) / sv_rect.height()).clamp(0.0, 1.0);
                }
            }

            // Crosshair indicator on current (s, v)
            let cx = sv_rect.left() + hsva.s * sv_rect.width();
            let cy = sv_rect.top() + (1.0 - hsva.v) * sv_rect.height();
            ui.painter().circle(pos2(cx, cy), 4.5, Color32::TRANSPARENT, Stroke::new(1.8, Color32::BLACK));
            ui.painter().circle(pos2(cx, cy), 3.5, Color32::TRANSPARENT, Stroke::new(1.5, Color32::WHITE));

            // Row 3: Full-width Hue spectrum bar
            let (hue_rect, hue_resp) = ui.allocate_exact_size(vec2(full_w, 13.0), Sense::click_and_drag());
            let stops: [(f32, Color32); 7] = [
                (0.0, Color32::from_rgb(255, 0, 0)),
                (0.1666, Color32::from_rgb(255, 255, 0)),
                (0.3333, Color32::from_rgb(0, 255, 0)),
                (0.5, Color32::from_rgb(0, 255, 255)),
                (0.6666, Color32::from_rgb(0, 0, 255)),
                (0.8333, Color32::from_rgb(255, 0, 255)),
                (1.0, Color32::from_rgb(255, 0, 0)),
            ];
            let mut h_mesh = egui::Mesh::default();
            for i in 0..6 {
                let (p0, c0) = stops[i];
                let (p1, c1) = stops[i + 1];
                let x0 = hue_rect.left() + p0 * hue_rect.width();
                let x1 = hue_rect.left() + p1 * hue_rect.width();
                let v_start = h_mesh.vertices.len() as u32;
                h_mesh.colored_vertex(pos2(x0, hue_rect.top()), c0);
                h_mesh.colored_vertex(pos2(x1, hue_rect.top()), c1);
                h_mesh.colored_vertex(pos2(x0, hue_rect.bottom()), c0);
                h_mesh.colored_vertex(pos2(x1, hue_rect.bottom()), c1);
                h_mesh.add_triangle(v_start, v_start + 1, v_start + 2);
                h_mesh.add_triangle(v_start + 2, v_start + 1, v_start + 3);
            }
            ui.painter().add(egui::Shape::mesh(h_mesh));
            ui.painter().rect_stroke(hue_rect, CornerRadius::same(2), t.hairline(), egui::StrokeKind::Inside);

            if hue_resp.clicked() || hue_resp.dragged() {
                if let Some(pos) = pointer_pos {
                    hsva.h = ((pos.x - hue_rect.left()) / hue_rect.width()).clamp(0.0, 1.0);
                }
            }

            let h_thumb_x = hue_rect.left() + hsva.h * hue_rect.width();
            ui.painter().rect(
                Rect::from_center_size(pos2(h_thumb_x, hue_rect.center().y), vec2(5.0, 15.0)),
                CornerRadius::same(2),
                Color32::WHITE,
                Stroke::new(1.0, Color32::BLACK),
                egui::StrokeKind::Middle,
            );

            // Row 4: Full-width Alpha bar (if has_alpha)
            if has_alpha {
                let (alpha_rect, alpha_resp) = ui.allocate_exact_size(vec2(full_w, 13.0), Sense::click_and_drag());
                let half_sq = 4.0;
                let mut bx = alpha_rect.left();
                let mut row = 0;
                while bx < alpha_rect.right() {
                    let mut by = alpha_rect.top();
                    let mut col = 0;
                    while by < alpha_rect.bottom() {
                        let fill = if (row + col) % 2 == 0 { Color32::from_gray(60) } else { Color32::from_gray(100) };
                        let r = Rect::from_min_size(pos2(bx, by), vec2(half_sq, half_sq)).intersect(alpha_rect);
                        ui.painter().rect_filled(r, CornerRadius::ZERO, fill);
                        by += half_sq;
                        col += 1;
                    }
                    bx += half_sq;
                    row += 1;
                }

                let col_0 = Color32::from(egui::ecolor::Hsva::new(hsva.h, hsva.s, hsva.v, 0.0));
                let col_1 = Color32::from(egui::ecolor::Hsva::new(hsva.h, hsva.s, hsva.v, 1.0));
                let mut a_mesh = egui::Mesh::default();
                a_mesh.colored_vertex(pos2(alpha_rect.left(), alpha_rect.top()), col_0);
                a_mesh.colored_vertex(pos2(alpha_rect.right(), alpha_rect.top()), col_1);
                a_mesh.colored_vertex(pos2(alpha_rect.left(), alpha_rect.bottom()), col_0);
                a_mesh.colored_vertex(pos2(alpha_rect.right(), alpha_rect.bottom()), col_1);
                a_mesh.add_triangle(0, 1, 2);
                a_mesh.add_triangle(2, 1, 3);
                ui.painter().add(egui::Shape::mesh(a_mesh));
                ui.painter().rect_stroke(alpha_rect, CornerRadius::same(2), t.hairline(), egui::StrokeKind::Inside);

                if alpha_resp.clicked() || alpha_resp.dragged() {
                    if let Some(pos) = pointer_pos {
                        hsva.a = ((pos.x - alpha_rect.left()) / alpha_rect.width()).clamp(0.0, 1.0);
                    }
                }

                let a_thumb_x = alpha_rect.left() + hsva.a * alpha_rect.width();
                ui.painter().rect(
                    Rect::from_center_size(pos2(a_thumb_x, alpha_rect.center().y), vec2(5.0, 15.0)),
                    CornerRadius::same(2),
                    Color32::WHITE,
                    Stroke::new(1.0, Color32::BLACK),
                    egui::StrokeKind::Middle,
                );
            }

            // Sync updated HSVA back to RGBA
            let c32 = Color32::from(hsva);
            color[0] = c32.r();
            color[1] = c32.g();
            color[2] = c32.b();
            if has_alpha {
                color[3] = (hsva.a * 255.0).round() as u8;
            }

            ui.ctx().data_mut(|d| {
                d.insert_temp(hsva_id, hsva);
                d.insert_temp(last_rgba_id, *color);
            });
        });
}
