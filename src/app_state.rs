use std::path::PathBuf;
use image::imageops;
use serde_json::{json, Value};

use crate::camera::{
    compute_crop_box, compute_target_crop_box, crop_and_resize, crop_and_resize_preview,
    draw_biometric_guides, list_available_cameras, to_jpeg_base64,
    CameraManager, CropBox, FaceDetectorWorker, MacFaceRect,
};
use crate::config::Config;
use crate::edevlet::save_capture;

pub struct AppState {
    pub cfg: Config,
    pub camera: CameraManager,
    pub face_worker: FaceDetectorWorker,
    pub count: u32,
    pub last_name: String,
    pub last_path: String,
    pub status: String,
    pub last_target_b64: Option<String>,
    pub target_window_open: bool,
    pub smooth_box: Option<CropBox>,
    pub frame_counter: u32,
    pub last_face: Option<MacFaceRect>,
    pub last_face_found: bool,
    pub faces_detected_count: u32,
    pub consecutive_misses: u32,
}

impl AppState {
    pub fn new() -> Self {
        let cfg = Config::load();
        Self {
            cfg,
            camera: CameraManager::new(),
            face_worker: FaceDetectorWorker::new(),
            count: 0,
            last_name: String::new(),
            last_path: String::new(),
            status: "Hazır.".to_string(),
            last_target_b64: None,
            target_window_open: false,
            smooth_box: None,
            frame_counter: 0,
            last_face: None,
            last_face_found: false,
            faces_detected_count: 0,
            consecutive_misses: 0,
        }
    }

    pub fn handle_rpc(&mut self, method: &str, args: &[Value]) -> Value {
        match method {
            "get_state" => self.get_state(),
            "set_zoom" => {
                let z = args.get(0).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
                self.set_zoom(z)
            }
            "set_face_detect" => {
                let enabled = args.get(0).and_then(|v| v.as_bool()).unwrap_or(true);
                self.set_face_detect(enabled)
            }
            "set_face_detect_every" => {
                let every = args.get(0).and_then(|v| v.as_u64()).unwrap_or(3) as u32;
                self.set_face_detect_every(every)
            }
            "start_camera" => self.start_camera(),
            "stop_camera" => self.stop_camera(),
            "toggle_camera" => self.toggle_camera(),
            "get_snapshot" => self.get_snapshot(),
            "get_target_snapshot" => self.get_target_snapshot(),
            "capture_now" => {
                let name = args.get(0).and_then(|v| v.as_str()).unwrap_or("");
                self.capture_now(name)
            }
            "apply_settings" => {
                if let Some(val) = args.get(0) {
                    self.apply_settings(val)
                } else {
                    json!({"ok": false, "message": "Ayar verisi eksik"})
                }
            }
            "open_folder" => self.open_folder(),
            "pick_folder" => self.pick_folder(),
            "get_cameras" => json!({"ok": true, "cameras": list_available_cameras()}),
            "open_target_window" => {
                self.target_window_open = true;
                json!({"ok": true, "opened": true})
            }
            "close_target_window" => {
                self.target_window_open = false;
                json!({"ok": true})
            }
            "is_target_window_open" => {
                json!({"open": self.target_window_open})
            }
            "exit_app" => {
                let _ = self.stop_camera();
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(60));
                    std::process::exit(0);
                });
                json!({"ok": true})
            }
            _ => json!({"ok": false, "message": format!("Bilinmeyen metot: {}", method)}),
        }
    }

    pub fn get_state(&self) -> Value {
        let out_dir_display = self.cfg.output_dir.to_string_lossy().to_string();
        let cameras = list_available_cameras();
        json!({
            "camera_index": self.cfg.camera_index,
            "cameras": cameras,
            "cam_resolution": self.cfg.cam_resolution,
            "width": self.cfg.width,
            "height": self.cfg.height,
            "image_format": self.cfg.image_format,
            "jpeg_quality": self.cfg.jpeg_quality,
            "edevlet_limit": self.cfg.edevlet_limit,
            "output_dir": out_dir_display,
            "face_detect": self.cfg.face_detect,
            "face_head_ratio": self.cfg.face_head_ratio,
            "face_v_offset": self.cfg.face_v_offset,
            "face_smoothing": self.cfg.face_smoothing,
            "face_detect_every": self.cfg.face_detect_every,
            "frame_zoom": self.cfg.frame_zoom,
            "running": self.camera.is_running(),
            "count": self.count,
            "last_name": self.last_name,
            "last_path": self.last_path,
            "status": self.status,
        })
    }

    pub fn set_zoom(&mut self, zoom: f32) -> Value {
        let z = zoom.clamp(0.4, 2.5);
        let rounded = (z * 100.0).round() / 100.0;
        self.cfg.frame_zoom = rounded;
        let _ = self.cfg.update_env_file(&crate::config::Config::get_env_path());
        json!({"ok": true, "zoom": rounded})
    }

    pub fn set_face_detect(&mut self, enabled: bool) -> Value {
        self.cfg.face_detect = enabled;
        if !enabled {
            self.last_face = None;
            self.last_face_found = false;
            self.faces_detected_count = 0;
            self.consecutive_misses = 0;
        }
        let _ = self.cfg.update_env_file(&crate::config::Config::get_env_path());
        json!({"ok": true, "face_detect": enabled})
    }

    pub fn set_face_detect_every(&mut self, every: u32) -> Value {
        let e = every.clamp(1, 60);
        self.cfg.face_detect_every = e;
        let _ = self.cfg.update_env_file(&crate::config::Config::get_env_path());
        json!({"ok": true, "face_detect_every": e})
    }

    pub fn start_camera(&mut self) -> Value {
        match self.camera.start(self.cfg.camera_index, &self.cfg.cam_resolution) {
            Ok(idx) => {
                self.status = format!("Kamera {} açıldı.", idx);
                self.cfg.camera_index = idx;
                json!({
                    "ok": true,
                    "running": true,
                    "message": self.status,
                    "camera_index": idx
                })
            }
            Err(e) => {
                self.status = format!("Hata: kamera açılamadı ({})", e);
                json!({
                    "ok": false,
                    "running": false,
                    "message": self.status
                })
            }
        }
    }

    pub fn stop_camera(&mut self) -> Value {
        self.camera.stop();
        self.status = "Kamera durduruldu.".to_string();
        json!({
            "ok": true,
            "running": false,
            "message": self.status
        })
    }

    pub fn toggle_camera(&mut self) -> Value {
        if self.camera.is_running() {
            self.stop_camera()
        } else {
            self.start_camera()
        }
    }

    pub fn get_snapshot(&mut self) -> Value {
        if !self.camera.is_running() {
            return json!({
                "live": null,
                "target": null,
                "running": false,
                "count": self.count,
                "last_name": self.last_name,
                "status": self.status,
                "zoom": self.cfg.frame_zoom,
            });
        }

        let frame_arc = match self.camera.read_frame() {
            Some(f) => f,
            None => {
                return json!({
                    "live": null,
                    "target": self.last_target_b64,
                    "running": true,
                    "count": self.count,
                    "last_name": self.last_name,
                    "status": "Kamera hazırlanıyor...",
                    "zoom": self.cfg.frame_zoom,
                });
            }
        };
        let frame: &image::RgbImage = &frame_arc;

        // Yüz algılama mantığı (Donanım hızlandırmalı)
        self.frame_counter = self.frame_counter.wrapping_add(1);

        if self.cfg.face_detect {
            let every = self.cfg.face_detect_every.max(1);
            if self.frame_counter % every == 0 {
                self.face_worker.submit_frame(std::sync::Arc::clone(&frame_arc));
            }
            let (detected, num_faces) = self.face_worker.get_face();
            self.faces_detected_count = num_faces;
            if let Some(f) = detected {
                self.last_face = Some(f);
                self.last_face_found = true;
                self.consecutive_misses = 0;
            } else {
                self.consecutive_misses += 1;
                if self.consecutive_misses >= 5 {
                    self.last_face = None;
                    self.last_face_found = false;
                    self.faces_detected_count = 0;
                }
            }
        } else {
            self.last_face = None;
            self.last_face_found = false;
            self.faces_detected_count = 0;
            self.consecutive_misses = 0;
        }

        let (target_box, face_detected) = compute_target_crop_box(
            frame.width(),
            frame.height(),
            self.cfg.width,
            self.cfg.height,
            self.cfg.frame_zoom,
            if self.cfg.face_detect { self.last_face.as_ref() } else { None },
            self.cfg.face_head_ratio,
            self.cfg.face_v_offset,
        );

        // Yumuşak geçiş filtresi (Pürüzsüz takip)
        let smoothing = if face_detected {
            self.cfg.face_smoothing.clamp(0.05, 0.95)
        } else {
            0.25
        };

        let box_info = match self.smooth_box {
            Some(prev) => {
                let next = CropBox {
                    x: prev.x * (1.0 - smoothing) + target_box.x * smoothing,
                    y: prev.y * (1.0 - smoothing) + target_box.y * smoothing,
                    w: prev.w * (1.0 - smoothing) + target_box.w * smoothing,
                    h: prev.h * (1.0 - smoothing) + target_box.h * smoothing,
                };
                self.smooth_box = Some(next);
                next
            }
            None => {
                self.smooth_box = Some(target_box);
                target_box
            }
        };

        // Hedef önizleme görüntüsü (133x171) - Hızlı Triangle (bilinear) filtre ve JPEG ile
        let target_img = crop_and_resize_preview(&frame, &box_info, self.cfg.width, self.cfg.height);
        let target_b64 = to_jpeg_base64(&target_img, 85);
        self.last_target_b64 = target_b64.clone();

        // Canlı önizleme: Hızlı 640px ölçekleme ve kılavuz çizgileri
        let target_w = 640;
        let scale = target_w as f32 / frame.width() as f32;
        let mut target_h = (frame.height() as f32 * scale).round() as u32;
        if target_h % 2 != 0 {
            target_h += 1;
        }

        let mut live_img = imageops::resize(frame, target_w, target_h, imageops::FilterType::Nearest);
        let scaled_box = CropBox {
            x: box_info.x * scale,
            y: box_info.y * scale,
            w: box_info.w * scale,
            h: box_info.h * scale,
        };
        draw_biometric_guides(&mut live_img, &scaled_box);
        let live_b64 = to_jpeg_base64(&live_img, 65);

        let box_size = format!("{}x{}", box_info.w.round() as u32, box_info.h.round() as u32);

        json!({
            "live": live_b64,
            "target": target_b64,
            "running": true,
            "face": self.last_face_found,
            "face_count": self.faces_detected_count,
            "face_detect": self.cfg.face_detect,
            "face_detect_every": self.cfg.face_detect_every,
            "box_size": box_size,
            "count": self.count,
            "last_name": self.last_name,
            "status": self.status,
            "zoom": self.cfg.frame_zoom,
        })
    }

    pub fn get_target_snapshot(&mut self) -> Value {
        if !self.camera.is_running() {
            return json!({
                "target": null,
                "running": false,
                "face": false,
                "face_count": 0,
                "face_detect": self.cfg.face_detect,
                "face_detect_every": self.cfg.face_detect_every,
                "box_size": "—",
                "count": self.count,
                "last_name": self.last_name,
                "status": self.status,
                "zoom": self.cfg.frame_zoom,
                "width": self.cfg.width,
                "height": self.cfg.height,
            });
        }

        self.get_snapshot();

        let (bw, bh) = if let Some(b) = self.smooth_box {
            (b.w.round() as u32, b.h.round() as u32)
        } else if let Some(f) = self.camera.read_frame() {
            let b = compute_crop_box(f.width(), f.height(), self.cfg.width, self.cfg.height, self.cfg.frame_zoom);
            (b.w.round() as u32, b.h.round() as u32)
        } else {
            (0, 0)
        };
        let box_size = if bw > 0 { format!("{}x{}", bw, bh) } else { "—".to_string() };

        json!({
            "target": self.last_target_b64,
            "running": self.camera.is_running(),
            "face": self.last_face_found,
            "face_count": self.faces_detected_count,
            "face_detect": self.cfg.face_detect,
            "face_detect_every": self.cfg.face_detect_every,
            "box_size": box_size,
            "count": self.count,
            "last_name": self.last_name,
            "status": self.status,
            "zoom": self.cfg.frame_zoom,
            "width": self.cfg.width,
            "height": self.cfg.height,
        })
    }

    pub fn capture_now(&mut self, filename: &str) -> Value {
        if !self.camera.is_running() {
            return json!({
                "ok": false,
                "message": "Yakalama için önce kamerayı başlatın."
            });
        }

        let frame_arc = match self.camera.read_frame() {
            Some(f) => f,
            None => {
                return json!({
                    "ok": false,
                    "message": "Kameradan kare alınamadı."
                });
            }
        };
        let frame: &image::RgbImage = &frame_arc;

        // Canlı vizördeki güncel takip kutusunu kullan
        let box_info = self.smooth_box.unwrap_or_else(|| {
            compute_crop_box(
                frame.width(),
                frame.height(),
                self.cfg.width,
                self.cfg.height,
                self.cfg.frame_zoom,
            )
        });

        let small = crop_and_resize(&frame, &box_info, self.cfg.width, self.cfg.height);

        let effective_dir = self.cfg.get_effective_output_dir();
        match save_capture(
            &small,
            &effective_dir,
            filename,
            &self.cfg.image_format,
            self.cfg.jpeg_quality,
            self.cfg.edevlet_limit,
        ) {
            Ok(path) => {
                self.count += 1;
                self.last_path = path.to_string_lossy().to_string();
                self.last_name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let size_kb = (size_bytes as f64 / 1024.0).round().max(1.0) as u32;

                let q_info = if self.cfg.image_format == "jpg" {
                    format!(", Q%{}", self.cfg.jpeg_quality)
                } else {
                    String::new()
                };

                self.status = format!(
                    "Kaydedildi: {} (merkezi kadraj{}, {} KB)",
                    self.last_name, q_info, size_kb
                );

                json!({
                    "ok": true,
                    "path": self.last_path,
                    "name": self.last_name,
                    "count": self.count,
                    "size_kb": size_kb,
                    "message": self.status,
                })
            }
            Err(e) => json!({
                "ok": false,
                "message": format!("Kayıt hatası: {}", e),
            }),
        }
    }

    pub fn apply_settings(&mut self, values: &Value) -> Value {
        let cam = values
            .get("camera_index")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.cfg.camera_index as u64) as u32;
        let cam_res = values
            .get("cam_resolution")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.cfg.cam_resolution)
            .trim()
            .to_lowercase();
        let w = values
            .get("width")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.cfg.width as u64) as u32;
        let h = values
            .get("height")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.cfg.height as u64) as u32;
        let fmt = values
            .get("image_format")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.cfg.image_format)
            .trim()
            .to_lowercase();
        let quality = values
            .get("jpeg_quality")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.cfg.jpeg_quality as u64) as u8;
        let edevlet = values
            .get("edevlet_limit")
            .and_then(|v| v.as_bool())
            .unwrap_or(self.cfg.edevlet_limit);
        let face = values
            .get("face_detect")
            .and_then(|v| v.as_bool())
            .unwrap_or(self.cfg.face_detect);
        let head = values
            .get("face_head_ratio")
            .and_then(|v| v.as_f64())
            .unwrap_or(self.cfg.face_head_ratio as f64) as f32;
        let voff = values
            .get("face_v_offset")
            .and_then(|v| v.as_f64())
            .unwrap_or(self.cfg.face_v_offset as f64) as f32;
        let smooth = values
            .get("face_smoothing")
            .and_then(|v| v.as_f64())
            .unwrap_or(self.cfg.face_smoothing as f64) as f32;
        let every = values
            .get("face_detect_every")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.cfg.face_detect_every as u64) as u32;
        let zoom = values
            .get("frame_zoom")
            .and_then(|v| v.as_f64())
            .unwrap_or(self.cfg.frame_zoom as f64) as f32;
        let out_raw = values
            .get("output_dir")
            .and_then(|v| v.as_str())
            .unwrap_or("captures")
            .trim();

        if !(1..=4096).contains(&w) || !(1..=4096).contains(&h) {
            return json!({"ok": false, "message": "Çözünürlük 1–4096 arasında olmalı"});
        }
        if !(10..=100).contains(&quality) {
            return json!({"ok": false, "message": "Kalite %10–100 arasında olmalı"});
        }
        if fmt != "png" && fmt != "jpg" {
            return json!({"ok": false, "message": "Format png veya jpg olmalı"});
        }

        let cam_changed = cam != self.cfg.camera_index;
        let cam_res_changed = cam_res != self.cfg.cam_resolution;

        self.cfg.camera_index = cam;
        self.cfg.cam_resolution = cam_res.clone();
        self.cfg.width = w;
        self.cfg.height = h;
        self.cfg.image_format = fmt.clone();
        self.cfg.jpeg_quality = quality;
        self.cfg.edevlet_limit = edevlet;
        self.cfg.output_dir = PathBuf::from(if out_raw.is_empty() { "captures" } else { out_raw });
        self.cfg.face_detect = face;
        self.cfg.face_head_ratio = head;
        self.cfg.face_v_offset = voff;
        self.cfg.face_smoothing = smooth;
        self.cfg.face_detect_every = every;
        self.cfg.frame_zoom = zoom;

        let _ = self.cfg.update_env_file(&crate::config::Config::get_env_path());

        if (cam_changed || cam_res_changed) && self.camera.is_running() {
            self.stop_camera();
            self.start_camera();
        }

        let q_txt = if fmt == "jpg" {
            format!(", Q%{}", quality)
        } else {
            String::new()
        };
        let msg = format!(
            "Ayarlar kaydedildi: {}x{}, {}{}, kamera {} ({}), {}",
            w, h, fmt.to_uppercase(), q_txt, cam, cam_res, out_raw
        );
        self.status = msg.clone();

        json!({
            "ok": true,
            "message": msg,
            "state": self.get_state()
        })
    }

    pub fn open_folder(&self) -> Value {
        let dir = self.cfg.get_effective_output_dir();
        let _ = std::fs::create_dir_all(&dir);
        match open::that(&dir) {
            Ok(_) => json!({"ok": true, "message": format!("Klasör açıldı: {:?}", dir)}),
            Err(e) => json!({"ok": false, "message": format!("Klasör açılamadı: {}", e)}),
        }
    }

    pub fn pick_folder(&self) -> Value {
        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
            json!({"ok": true, "path": folder.to_string_lossy().to_string()})
        } else {
            json!({"ok": false, "path": null})
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_state_snapshots() {
        let mut state = AppState::new();
        state.cfg.face_detect = true;
        state.cfg.face_detect_every = 2;
        let start_res = state.start_camera();
        println!("Start camera: {:?}", start_res);
        std::thread::sleep(std::time::Duration::from_millis(500));

        for i in 0..20 {
            let t0 = std::time::Instant::now();
            let snap = state.get_snapshot();
            let has_live = snap.get("live").map(|v| !v.is_null()).unwrap_or(false);
            println!("Snapshot {}: took {:?}, live: {}", i, t0.elapsed(), has_live);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        state.stop_camera();
    }
}
