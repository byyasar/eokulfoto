use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use image::{imageops, DynamicImage, ImageFormat, Rgb, RgbImage};
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution};
use nokhwa::Camera;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CameraDevice {
    pub index: u32,
    pub name: String,
}

pub fn list_available_cameras() -> Vec<CameraDevice> {
    use nokhwa::utils::ApiBackend;
    let mut devices = Vec::new();

    if let Ok(cams) = nokhwa::query(ApiBackend::Auto) {
        for (i, cam_info) in cams.into_iter().enumerate() {
            let idx = cam_info.index().as_index().unwrap_or(i as u32);
            let name = cam_info.human_name();
            devices.push(CameraDevice {
                index: idx,
                name: if name.trim().is_empty() {
                    format!("Kamera {}", idx)
                } else {
                    name
                },
            });
        }
    }

    if devices.is_empty() {
        devices.push(CameraDevice {
            index: 0,
            name: "Kamera 0 (Varsayılan)".to_string(),
        });
        devices.push(CameraDevice {
            index: 1,
            name: "Kamera 1 (Harici / USB)".to_string(),
        });
    }

    devices
}

#[derive(Debug, Clone, Copy)]
pub struct CropBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl CropBox {
    pub fn as_rect(&self) -> (i32, i32, u32, u32) {
        let x = (self.x - self.w / 2.0).round() as i32;
        let y = (self.y - self.h / 2.0).round() as i32;
        let w = self.w.round().max(1.0) as u32;
        let h = self.h.round().max(1.0) as u32;
        (x, y, w, h)
    }
}

enum CamCommand {
    Start {
        index: u32,
        res: String,
        resp: Sender<Result<u32, String>>,
    },
    Stop,
    Shutdown,
}

pub struct CameraManager {
    cmd_tx: Sender<CamCommand>,
    running: Arc<AtomicBool>,
    latest_frame: Arc<Mutex<Option<Arc<RgbImage>>>>,
    pub current_index: u32,
    _worker: Option<JoinHandle<()>>,
}

impl CameraManager {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = channel::<CamCommand>();
        let running = Arc::new(AtomicBool::new(false));
        let running_clone = Arc::clone(&running);
        let latest_frame = Arc::new(Mutex::new(None));
        let latest_frame_clone = Arc::clone(&latest_frame);

        let worker = thread::spawn(move || {
            let mut camera: Option<Camera> = None;

            loop {
                // Eğer çalışıyorsa ve kamera varsa kare oku
                if running_clone.load(Ordering::SeqCst) && camera.is_some() {
                    // Try to receive non-blocking commands
                    if let Ok(cmd) = cmd_rx.try_recv() {
                        match cmd {
                            CamCommand::Start { index, res, resp } => {
                                let res = Self::open_camera_internal(&mut camera, index, &res);
                                let ok = res.is_ok();
                                running_clone.store(ok, Ordering::SeqCst);
                                let _ = resp.send(res);
                            }
                            CamCommand::Stop => {
                                if let Some(mut cam) = camera.take() {
                                    let _ = cam.stop_stream();
                                }
                                running_clone.store(false, Ordering::SeqCst);
                                *latest_frame_clone.lock().unwrap() = None;
                            }
                            CamCommand::Shutdown => {
                                if let Some(mut cam) = camera.take() {
                                    let _ = cam.stop_stream();
                                }
                                break;
                            }
                        }
                    }

                    if let Some(cam) = camera.as_mut() {
                        match cam.frame() {
                            Ok(frame_buffer) => {
                                if let Ok(img) = frame_buffer.decode_image::<RgbFormat>() {
                                    *latest_frame_clone.lock().unwrap() = Some(Arc::new(img));
                                }
                            }
                            Err(_) => {
                                thread::sleep(std::time::Duration::from_millis(5));
                            }
                        }
                    } else {
                        thread::sleep(std::time::Duration::from_millis(15));
                    }
                } else {
                    // Kamera kapalıyken komut bekle
                    match cmd_rx.recv() {
                        Ok(CamCommand::Start { index, res, resp }) => {
                            let res = Self::open_camera_internal(&mut camera, index, &res);
                            let ok = res.is_ok();
                            running_clone.store(ok, Ordering::SeqCst);
                            let _ = resp.send(res);
                        }
                        Ok(CamCommand::Stop) => {
                            if let Some(mut cam) = camera.take() {
                                let _ = cam.stop_stream();
                            }
                            running_clone.store(false, Ordering::SeqCst);
                            *latest_frame_clone.lock().unwrap() = None;
                        }
                        Ok(CamCommand::Shutdown) | Err(_) => {
                            if let Some(mut cam) = camera.take() {
                                let _ = cam.stop_stream();
                            }
                            break;
                        }
                    }
                }
            }
        });

        Self {
            cmd_tx,
            running,
            latest_frame,
            current_index: 0,
            _worker: Some(worker),
        }
    }

    fn open_camera_internal(camera: &mut Option<Camera>, index: u32, res_str: &str) -> Result<u32, String> {
        if let Some(mut cam) = camera.take() {
            let _ = cam.stop_stream();
        }

        let (cw, ch) = Self::parse_resolution(res_str);
        let mut candidates = Vec::new();

        if cw > 0 && ch > 0 {
            candidates.push(RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(
                CameraFormat::new(Resolution::new(cw, ch), FrameFormat::MJPEG, 30),
            )));
            candidates.push(RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(
                CameraFormat::new(Resolution::new(cw, ch), FrameFormat::YUYV, 30),
            )));
        }

        candidates.push(RequestedFormat::new::<RgbFormat>(
            RequestedFormatType::HighestFrameRate(30),
        ));
        candidates.push(RequestedFormat::new::<RgbFormat>(RequestedFormatType::None));

        let mut opened_cam: Option<Camera> = None;

        for req in &candidates {
            if let Ok(c) = Camera::new(CameraIndex::Index(index), req.clone()) {
                opened_cam = Some(c);
                break;
            }
        }

        // İstenen indeks açılamadıysa ve 0 değilse 0'ı dene
        if opened_cam.is_none() && index != 0 {
            if let Ok(c) = Camera::new(CameraIndex::Index(0), RequestedFormat::new::<RgbFormat>(RequestedFormatType::None)) {
                opened_cam = Some(c);
            }
        }

        let mut c = opened_cam.ok_or_else(|| "Kamera açılamadı: uygun kamera aygıtı bulunamadı".to_string())?;
        c.open_stream().map_err(|e| format!("Kamera akışı başlatılamadı: {}", e))?;
        *camera = Some(c);
        Ok(index)
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn start(&mut self, camera_index: u32, resolution_str: &str) -> Result<u32, String> {
        let (resp_tx, resp_rx) = channel();
        let _ = self.cmd_tx.send(CamCommand::Start {
            index: camera_index,
            res: resolution_str.to_string(),
            resp: resp_tx,
        });

        match resp_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(idx)) => {
                self.current_index = idx;
                Ok(idx)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err("Kamera açma zaman aşımına uğradı (kamera başka bir uygulama tarafından kullanılıyor olabilir)".to_string()),
        }
    }

    pub fn stop(&mut self) {
        let _ = self.cmd_tx.send(CamCommand::Stop);
    }

    pub fn read_frame(&self) -> Option<Arc<RgbImage>> {
        self.latest_frame.lock().unwrap().clone()
    }

    pub fn parse_resolution(val: &str) -> (u32, u32) {
        let v = val.trim().to_lowercase();
        if let Some((w_str, h_str)) = v.split_once('x') {
            if let (Ok(w), Ok(h)) = (w_str.trim().parse(), h_str.trim().parse()) {
                return (w, h);
            }
        }
        (0, 0)
    }
}

impl Drop for CameraManager {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(CamCommand::Shutdown);
    }
}

pub struct FaceDetectorWorker {
    frame_tx: std::sync::mpsc::SyncSender<Arc<RgbImage>>,
    result: Arc<Mutex<(Option<MacFaceRect>, u32)>>,
}

impl FaceDetectorWorker {
    pub fn new() -> Self {
        let (frame_tx, frame_rx) = std::sync::mpsc::sync_channel::<Arc<RgbImage>>(1);
        let result = Arc::new(Mutex::new((None, 0)));
        let result_clone = Arc::clone(&result);

        thread::spawn(move || {
            #[cfg(target_os = "windows")]
            unsafe {
                extern "system" {
                    fn CoInitializeEx(pv_reserved: *const std::ffi::c_void, dw_co_init: u32) -> i32;
                }
                let _ = CoInitializeEx(std::ptr::null(), 0); // COINIT_MULTITHREADED (MTA)
            }

            while let Ok(frame) = frame_rx.recv() {
                // Hızlı yüz tespiti için küçük JPEG kare oluştur (~360px genişlik)
                let small_w = 360;
                let scale = small_w as f32 / frame.width() as f32;
                let small_h = ((frame.height() as f32 * scale).round() as u32).max(1);
                let small_img = imageops::resize(&*frame, small_w, small_h, imageops::FilterType::Nearest);

                let mut jpeg_bytes = Vec::new();
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, 60);
                if encoder.encode_image(&image::DynamicImage::ImageRgb8(small_img)).is_ok() {
                    let res = detect_face_native(&jpeg_bytes);
                    if let Ok(mut lock) = result_clone.lock() {
                        *lock = res;
                    }
                }
            }
        });

        Self { frame_tx, result }
    }

    pub fn submit_frame(&self, frame: Arc<RgbImage>) {
        let _ = self.frame_tx.try_send(frame);
    }

    pub fn get_face(&self) -> (Option<MacFaceRect>, u32) {
        self.result.lock().map(|l| *l).unwrap_or((None, 0))
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MacFaceRect {
    pub found: bool,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[cfg(target_os = "macos")]
extern "C" {
    fn mac_detect_face_in_jpeg(data: *const u8, len: usize) -> MacFaceRect;
}

pub fn detect_face_native(_jpeg_bytes: &[u8]) -> (Option<MacFaceRect>, u32) {
    #[cfg(target_os = "macos")]
    {
        if _jpeg_bytes.is_empty() {
            return (None, 0);
        }
        let res = unsafe { mac_detect_face_in_jpeg(_jpeg_bytes.as_ptr(), _jpeg_bytes.len()) };
        if res.found {
            (Some(res), 1)
        } else {
            (None, 0)
        }
    }
    #[cfg(target_os = "windows")]
    {
        win_face::detect_face_windows(_jpeg_bytes)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        (None, 0)
    }
}

#[cfg(target_os = "windows")]
mod win_face {
    use super::MacFaceRect;
    use std::sync::{Mutex, OnceLock};
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
    use windows::Graphics::Imaging::{BitmapDecoder, BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::FaceAnalysis::FaceDetector;

    static DETECTOR: OnceLock<Option<Mutex<FaceDetector>>> = OnceLock::new();

    fn get_detector() -> Option<&'static Mutex<FaceDetector>> {
        DETECTOR.get_or_init(|| {
            let op = FaceDetector::CreateAsync().ok()?;
            let detector = op.join().ok()?;
            Some(Mutex::new(detector))
        }).as_ref()
    }

    pub fn detect_face_windows(jpeg_bytes: &[u8]) -> (Option<MacFaceRect>, u32) {
        if jpeg_bytes.is_empty() {
            return (None, 0);
        }

        let Some(detector_lock) = get_detector() else { return (None, 0); };
        let Ok(detector) = detector_lock.lock() else { return (None, 0); };

        // Memory stream oluştur
        let Ok(stream) = InMemoryRandomAccessStream::new() else { return (None, 0); };
        let Ok(writer) = DataWriter::CreateDataWriter(&stream) else { return (None, 0); };
        if writer.WriteBytes(jpeg_bytes).is_err() { return (None, 0); }
        if let Ok(store_op) = writer.StoreAsync() {
            let _ = store_op.join();
        }
        let _ = writer.DetachStream();
        let _ = stream.Seek(0);

        let Ok(decoder_op) = BitmapDecoder::CreateAsync(&stream) else { return (None, 0); };
        let Ok(decoder) = decoder_op.join() else { return (None, 0); };
        let Ok(bitmap_op) = decoder.GetSoftwareBitmapAsync() else { return (None, 0); };
        let Ok(bitmap) = bitmap_op.join() else { return (None, 0); };

        let Ok(fmt) = bitmap.BitmapPixelFormat() else { return (None, 0); };
        let detect_bitmap = if FaceDetector::IsBitmapPixelFormatSupported(fmt).unwrap_or(false) {
            bitmap
        } else {
            match SoftwareBitmap::Convert(&bitmap, BitmapPixelFormat::Gray8) {
                Ok(b) => b,
                Err(_) => return (None, 0),
            }
        };

        let pixel_w = match detect_bitmap.PixelWidth() {
            Ok(w) if w > 0 => w as f32,
            _ => return (None, 0),
        };
        let pixel_h = match detect_bitmap.PixelHeight() {
            Ok(h) if h > 0 => h as f32,
            _ => return (None, 0),
        };

        let Ok(faces_op) = detector.DetectFacesAsync(&detect_bitmap) else { return (None, 0); };
        let Ok(faces) = faces_op.join() else { return (None, 0); };
        let count = faces.Size().unwrap_or(0);
        if count == 0 {
            return (None, 0);
        }

        // En büyük yüzü bul
        let mut best: Option<MacFaceRect> = None;
        let mut max_area = 0.0f32;

        for i in 0..count {
            if let Ok(face) = faces.GetAt(i) {
                if let Ok(box_bounds) = face.FaceBox() {
                    let w = box_bounds.Width as f32 / pixel_w;
                    let h = box_bounds.Height as f32 / pixel_h;
                    let x = box_bounds.X as f32 / pixel_w;
                    let y = box_bounds.Y as f32 / pixel_h;
                    let area = w * h;
                    if area > max_area {
                        max_area = area;
                        best = Some(MacFaceRect {
                            found: true,
                            x,
                            y,
                            width: w,
                            height: h,
                        });
                    }
                }
            }
        }

        (best, count)
    }
}

pub fn compute_target_crop_box(
    frame_w: u32,
    frame_h: u32,
    out_w: u32,
    out_h: u32,
    zoom: f32,
    face: Option<&MacFaceRect>,
    head_ratio: f32,
    v_offset: f32,
) -> (CropBox, bool) {
    let fw = frame_w as f32;
    let fh = frame_h as f32;
    let out_aspect = (out_w as f32) / (out_h as f32);
    let z = zoom.clamp(0.4, 3.0);

    let max_crop_h = fh.min(fw / out_aspect);

    if let Some(f) = face {
        // Yüzün merkez ve boyutları
        let face_cx = (f.x + f.width / 2.0) * fw;
        let face_h = (f.height * fh).max(20.0);

        // e-Devlet ve ICAO biyometrik oran (~0.65)
        let hr = head_ratio.clamp(0.3, 0.95);
        let ideal_crop_h = ((face_h / hr) / z).clamp(24.0, max_crop_h);
        let crop_w = ideal_crop_h * out_aspect;

        // Dikey hizalama: Başın tepesindeki boşluk
        let face_top = f.y * fh;
        let head_top_est = (face_top - (face_h * 0.12)).max(0.0);
        let vo = v_offset.clamp(0.02, 0.35);
        let desired_top = head_top_est - (ideal_crop_h * vo);
        let target_cy = desired_top + ideal_crop_h / 2.0;

        let half_w = crop_w / 2.0;
        let half_h = ideal_crop_h / 2.0;
        let clamped_cx = face_cx.clamp(half_w, (fw - half_w).max(half_w));
        let clamped_cy = target_cy.clamp(half_h, (fh - half_h).max(half_h));

        (
            CropBox {
                x: clamped_cx,
                y: clamped_cy,
                w: crop_w,
                h: ideal_crop_h,
            },
            true,
        )
    } else {
        let base_h = fh * 0.72;
        let crop_h = (base_h / z).clamp(24.0, max_crop_h);
        let crop_w = crop_h * out_aspect;

        let cx = fw / 2.0;
        let cy = fh / 2.0;

        let half_w = crop_w / 2.0;
        let half_h = crop_h / 2.0;
        let clamped_cx = cx.clamp(half_w, (fw - half_w).max(half_w));
        let clamped_cy = cy.clamp(half_h, (fh - half_h).max(half_h));

        (
            CropBox {
                x: clamped_cx,
                y: clamped_cy,
                w: crop_w,
                h: crop_h,
            },
            false,
        )
    }
}

pub fn compute_crop_box(frame_w: u32, frame_h: u32, out_w: u32, out_h: u32, zoom: f32) -> CropBox {
    compute_target_crop_box(frame_w, frame_h, out_w, out_h, zoom, None, 0.65, 0.10).0
}

pub fn crop_and_resize(
    frame: &RgbImage,
    box_info: &CropBox,
    out_w: u32,
    out_h: u32,
) -> RgbImage {
    let (rx, ry, rw, rh) = box_info.as_rect();
    let img_w = frame.width() as i32;
    let img_h = frame.height() as i32;

    let safe_x = rx.clamp(0, (img_w - 1).max(0)) as u32;
    let safe_y = ry.clamp(0, (img_h - 1).max(0)) as u32;
    let safe_w = rw.min(frame.width().saturating_sub(safe_x)).max(1);
    let safe_h = rh.min(frame.height().saturating_sub(safe_y)).max(1);

    let cropped = imageops::crop_imm(frame, safe_x, safe_y, safe_w, safe_h).to_image();
    imageops::resize(&cropped, out_w, out_h, imageops::FilterType::Lanczos3)
}

pub fn crop_and_resize_preview(
    frame: &RgbImage,
    box_info: &CropBox,
    out_w: u32,
    out_h: u32,
) -> RgbImage {
    let (rx, ry, rw, rh) = box_info.as_rect();
    let img_w = frame.width() as i32;
    let img_h = frame.height() as i32;

    let safe_x = rx.clamp(0, (img_w - 1).max(0)) as u32;
    let safe_y = ry.clamp(0, (img_h - 1).max(0)) as u32;
    let safe_w = rw.min(frame.width().saturating_sub(safe_x)).max(1);
    let safe_h = rh.min(frame.height().saturating_sub(safe_y)).max(1);

    let cropped = imageops::crop_imm(frame, safe_x, safe_y, safe_w, safe_h).to_image();
    imageops::resize(&cropped, out_w, out_h, imageops::FilterType::Triangle)
}

fn draw_dashed_horiz_line(
    img: &mut RgbImage,
    y: i32,
    x_start: i32,
    x_end: i32,
    color: Rgb<u8>,
    dash_len: i32,
) {
    if y < 0 || y >= img.height() as i32 {
        return;
    }
    let start = x_start.max(0);
    let end = x_end.min(img.width() as i32);
    let mut x = start;
    let mut draw = true;
    while x < end {
        let next = (x + dash_len).min(end);
        if draw {
            for px in x..next {
                img.put_pixel(px as u32, y as u32, color);
            }
        }
        draw = !draw;
        x = next;
    }
}

fn draw_thick_line_h(img: &mut RgbImage, y: i32, x_start: i32, x_end: i32, color: Rgb<u8>, th: i32) {
    for dy in 0..th {
        let py = y + dy;
        if py >= 0 && py < img.height() as i32 {
            for px in x_start.max(0)..x_end.min(img.width() as i32) {
                img.put_pixel(px as u32, py as u32, color);
            }
        }
    }
}

fn draw_thick_line_v(img: &mut RgbImage, x: i32, y_start: i32, y_end: i32, color: Rgb<u8>, th: i32) {
    for dx in 0..th {
        let px = x + dx;
        if px >= 0 && px < img.width() as i32 {
            for py in y_start.max(0)..y_end.min(img.height() as i32) {
                img.put_pixel(px as u32, py as u32, color);
            }
        }
    }
}

pub fn draw_biometric_guides(img: &mut RgbImage, box_info: &CropBox) {
    let (x, y, bw, bh) = box_info.as_rect();
    let crop_color = Rgb([255, 215, 0]); // Canlı sarı / turuncu altın
    let guide_color = Rgb([0, 180, 220]); // Biyometrik mavi kılavuzlar

    // Çerçeve (2px kalınlık)
    draw_thick_line_h(img, y, x, x + bw as i32, crop_color, 2);
    draw_thick_line_h(img, y + bh as i32 - 2, x, x + bw as i32, crop_color, 2);
    draw_thick_line_v(img, x, y, y + bh as i32, crop_color, 2);
    draw_thick_line_v(img, x + bw as i32 - 2, y, y + bh as i32, crop_color, 2);

    // Vizör köşe vurguları (L çizgileri, 3px)
    let c_len = (bw.min(bh) as f32 * 0.12).round().clamp(10.0, 24.0) as i32;
    let th = 3;
    // Sol-üst
    draw_thick_line_h(img, y, x, x + c_len, crop_color, th);
    draw_thick_line_v(img, x, y, y + c_len, crop_color, th);
    // Sağ-üst
    draw_thick_line_h(img, y, x + bw as i32 - c_len, x + bw as i32, crop_color, th);
    draw_thick_line_v(img, x + bw as i32 - th, y, y + c_len, crop_color, th);
    // Sol-alt
    draw_thick_line_h(img, y + bh as i32 - th, x, x + c_len, crop_color, th);
    draw_thick_line_v(img, x, y + bh as i32 - c_len, y + bh as i32, crop_color, th);
    // Sağ-alt
    draw_thick_line_h(img, y + bh as i32 - th, x + bw as i32 - c_len, x + bw as i32, crop_color, th);
    draw_thick_line_v(img, x + bw as i32 - th, y + bh as i32 - c_len, y + bh as i32, crop_color, th);

    // Biyometrik hizalama kılavuzları
    let y_crown = y + (bh as f32 * 0.14) as i32;
    draw_dashed_horiz_line(img, y_crown, x + (bw as f32 * 0.20) as i32, x + (bw as f32 * 0.80) as i32, guide_color, 5);

    let y_eyes = y + (bh as f32 * 0.42) as i32;
    draw_dashed_horiz_line(img, y_eyes, x + (bw as f32 * 0.12) as i32, x + (bw as f32 * 0.88) as i32, guide_color, 5);

    let y_chin = y + (bh as f32 * 0.75) as i32;
    draw_dashed_horiz_line(img, y_chin, x + (bw as f32 * 0.22) as i32, x + (bw as f32 * 0.78) as i32, guide_color, 5);
}

pub fn to_jpeg_base64(img: &RgbImage, quality: u8) -> Option<String> {
    let mut buf = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
    if encoder.encode_image(&DynamicImage::ImageRgb8(img.clone())).is_ok() {
        Some(BASE64.encode(buf))
    } else {
        None
    }
}

#[allow(dead_code)]
pub fn to_png_base64(img: &RgbImage) -> Option<String> {
    let mut buf = Vec::new();
    let dyn_img = DynamicImage::ImageRgb8(img.clone());
    if dyn_img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png).is_ok() {
        Some(BASE64.encode(buf))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_face_detector() {
        let blank_img = RgbImage::new(320, 240);
        let mut buf = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 60);
        encoder.encode_image(&DynamicImage::ImageRgb8(blank_img)).unwrap();

        let (res, count) = detect_face_native(&buf);
        println!("Face detect result on blank frame: {:?}, count: {}", res, count);
        assert!(res.is_none());
        assert_eq!(count, 0);

        for i in 0..10 {
            let start = std::time::Instant::now();
            let (_r, c) = detect_face_native(&buf);
            println!("Run {}: {:?}, took {:?}", i, c, start.elapsed());
            assert_eq!(c, 0);
        }
    }

    #[test]
    fn test_camera_stream() {
        let mut mgr = CameraManager::new();
        let res = mgr.start(0, "auto");
        println!("Camera start result: {:?}", res);
        if res.is_ok() {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let mut last_sum: u64 = 0;
            for i in 0..20 {
                if let Some(frame) = mgr.read_frame() {
                    let sum: u64 = frame.pixels().take(100).map(|p| p[0] as u64).sum();
                    println!("Frame {}: changed = {}, sum = {}", i, sum != last_sum, sum);
                    last_sum = sum;
                } else {
                    println!("Frame {}: None", i);
                }
                std::thread::sleep(std::time::Duration::from_millis(60));
            }
            mgr.stop();
        }
    }
}


