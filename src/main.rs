mod app_state;
mod camera;
mod config;
mod edevlet;

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use clap::Parser;
use serde_json::{json, Value};
use tao::dpi::LogicalSize;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Window, WindowBuilder};
use tiny_http::{Header, Response, Server, StatusCode};
use wry::{WebView, WebViewBuilder};

use crate::app_state::AppState;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Webcam Capture — e-Devlet Biyometrik Fotoğraf Dönüştürücü (Rust)",
    long_about = None
)]
struct CliArgs {
    /// Tek kare yakala ve çık
    #[arg(long)]
    once: bool,

    /// Özel dosya adı
    #[arg(long, default_value = "")]
    name: String,

    /// Yalnızca yerel HTTP sunucusunu başlat (pencere açmadan)
    #[arg(long)]
    server_only: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum UserEvent {
    OpenTargetWindow,
    CloseTargetWindow,
}

mod assets {
    pub const INDEX_HTML: &str = include_str!("../webui/index.html");
    pub const TARGET_MONITOR_HTML: &str = include_str!("../webui/target_monitor.html");
    pub const STYLE_CSS: &str = include_str!("../webui/style.css");
    pub const APP_JS: &str = include_str!("../webui/app.js");
    pub const ICON_PNG: &[u8] = include_bytes!("../webui/icon.png");

    pub fn get_embedded_asset(path: &str, js_shim: &str) -> Option<(&'static str, Vec<u8>)> {
        let clean = path.trim_start_matches('/');
        match clean {
            "" | "index.html" => {
                let mut html = INDEX_HTML.to_string();
                if let Some(pos) = html.find("<head>") {
                    html.insert_str(pos + 6, js_shim);
                } else {
                    html.push_str(js_shim);
                }
                Some(("text/html; charset=utf-8", html.into_bytes()))
            }
            "target_monitor.html" => {
                let mut html = TARGET_MONITOR_HTML.to_string();
                if let Some(pos) = html.find("<head>") {
                    html.insert_str(pos + 6, js_shim);
                } else {
                    html.push_str(js_shim);
                }
                Some(("text/html; charset=utf-8", html.into_bytes()))
            }
            "style.css" => Some(("text/css; charset=utf-8", STYLE_CSS.as_bytes().to_vec())),
            "app.js" => Some(("application/javascript; charset=utf-8", APP_JS.as_bytes().to_vec())),
            "icon.png" => Some(("image/png", ICON_PNG.to_vec())),
            _ => None,
        }
    }
}

fn load_app_icon() -> Option<tao::window::Icon> {
    let img = image::load_from_memory(assets::ICON_PNG).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    tao::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}

#[cfg(target_os = "windows")]
mod portable_dlls {
    const WEBVIEW2_LOADER: &[u8] = include_bytes!("../res/WebView2Loader.dll");
    const LIBUNWIND: &[u8] = include_bytes!("../res/libunwind.dll");

    #[link(name = "kernel32")]
    extern "system" {
        fn SetDllDirectoryW(lpPathName: *const u16) -> i32;
        fn LoadLibraryW(lpLibFileName: *const u16) -> *mut std::ffi::c_void;
    }

    pub fn setup_embedded_dlls() {
        use std::os::windows::ffi::OsStrExt;

        let temp_dir = std::env::temp_dir().join("WebcamCapture_Runtime");
        let _ = std::fs::create_dir_all(&temp_dir);

        let webview_dll = temp_dir.join("WebView2Loader.dll");
        if !webview_dll.exists() || std::fs::metadata(&webview_dll).map(|m| m.len()).unwrap_or(0) != WEBVIEW2_LOADER.len() as u64 {
            let _ = std::fs::write(&webview_dll, WEBVIEW2_LOADER);
        }

        let unwind_dll = temp_dir.join("libunwind.dll");
        if !unwind_dll.exists() || std::fs::metadata(&unwind_dll).map(|m| m.len()).unwrap_or(0) != LIBUNWIND.len() as u64 {
            let _ = std::fs::write(&unwind_dll, LIBUNWIND);
        }

        let wide_path: Vec<u16> = temp_dir.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            SetDllDirectoryW(wide_path.as_ptr());
        }

        let wide_unwind: Vec<u16> = unwind_dll.as_os_str().encode_wide().chain(Some(0)).collect();
        let wide_wv: Vec<u16> = webview_dll.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            LoadLibraryW(wide_unwind.as_ptr());
            LoadLibraryW(wide_wv.as_ptr());
        }
    }
}

const JS_SHIM: &str = r#"
<script>
(function() {
  window.pywebview = {
    api: new Proxy({}, {
      get: function(target, prop) {
        return async function() {
          var args = Array.prototype.slice.call(arguments);
          try {
            var controller = new AbortController();
            var timeoutId = setTimeout(function() { controller.abort(); }, 4000);
            var res = await fetch("/api/rpc", {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({ method: prop, args: args }),
              signal: controller.signal
            });
            clearTimeout(timeoutId);
            return await res.json();
          } catch (e) {
            console.error("RPC Hatası:", e);
            throw e;
          }
        };
      }
    })
  };
})();
</script>
"#;

fn start_http_server(
    shared_state: Arc<Mutex<AppState>>,
    webui_dir: PathBuf,
    event_proxy: EventLoopProxy<UserEvent>,
) -> Result<u16, Box<dyn std::error::Error>> {
    let server = Arc::new(Server::http("127.0.0.1:0").map_err(|e| format!("HTTP sunucu başlatılamadı: {}", e))?);
    let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(48231);

    for _ in 0..4 {
        let server = Arc::clone(&server);
        let shared_state = Arc::clone(&shared_state);
        let webui_dir = webui_dir.clone();
        let event_proxy = event_proxy.clone();

        std::thread::spawn(move || {
            for mut req in server.incoming_requests() {
                let url_path = req.url().split('?').next().unwrap_or("/").to_string();

                if req.method() == &tiny_http::Method::Post && url_path == "/api/rpc" {
                    let mut body_str = String::new();
                    let max_len = req.body_length().unwrap_or(65536).min(65536) as u64;
                    use std::io::Read;
                    let _ = req.as_reader().take(max_len).read_to_string(&mut body_str);

                    let res_json = if let Ok(rpc) = serde_json::from_str::<Value>(&body_str) {
                        let method = rpc.get("method").and_then(|v| v.as_str()).unwrap_or("");
                        let empty_args = vec![];
                        let args = rpc.get("args").and_then(|v| v.as_array()).unwrap_or(&empty_args);

                        if method == "open_target_window" {
                            let _ = event_proxy.send_event(UserEvent::OpenTargetWindow);
                        } else if method == "close_target_window" {
                            let _ = event_proxy.send_event(UserEvent::CloseTargetWindow);
                        }

                        if method == "pick_folder" {
                            let res = std::thread::spawn(|| {
                                rfd::FileDialog::new().pick_folder()
                            }).join().ok().flatten();
                            if let Some(folder) = res {
                                let path_str = folder.to_string_lossy().to_string();
                                if let Ok(mut state) = shared_state.lock() {
                                    state.cfg.output_dir = folder;
                                }
                                json!({"ok": true, "path": path_str})
                            } else {
                                json!({"ok": false, "path": null})
                            }
                        } else {
                            let mut state = shared_state.lock().unwrap();
                            state.handle_rpc(method, args)
                        }
                    } else {
                        json!({"ok": false, "message": "Geçersiz JSON"})
                    };

                    let res_bytes = res_json.to_string().into_bytes();
                    let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..]).unwrap();
                    let cors = Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap();
                    let response = Response::from_data(res_bytes).with_header(header).with_header(cors);
                    let _ = req.respond(response);
                    continue;
                }

                // Statik dosyaları sunma
                let file_rel = if url_path == "/" || url_path.is_empty() {
                    "index.html"
                } else {
                    url_path.trim_start_matches('/')
                };

                let file_path = webui_dir.join(file_rel);
                if file_path.exists() && file_path.is_file() {
                    let ext = file_path.extension().and_then(|s| s.to_str()).unwrap_or("");
                    let content_type = match ext {
                        "html" => "text/html; charset=utf-8",
                        "css" => "text/css; charset=utf-8",
                        "js" => "application/javascript; charset=utf-8",
                        "png" => "image/png",
                        "ico" => "image/x-icon",
                        _ => "application/octet-stream",
                    };

                    let mut data = match fs::read(&file_path) {
                        Ok(d) => d,
                        Err(_) => {
                            let _ = req.respond(Response::empty(StatusCode(500)));
                            continue;
                        }
                    };

                    // HTML dosyalarına JS_SHIM enjekte et
                    if ext == "html" {
                        if let Ok(mut html_str) = fs::read_to_string(&file_path) {
                            if let Some(pos) = html_str.find("<head>") {
                                html_str.insert_str(pos + 6, JS_SHIM);
                            } else {
                                html_str.push_str(JS_SHIM);
                            }
                            data = html_str.into_bytes();
                        }
                    }

                    let ct_header = Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap();
                    let cors = Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap();
                    let response = Response::from_data(data).with_header(ct_header).with_header(cors);
                    let _ = req.respond(response);
                } else if let Some((content_type, data)) = assets::get_embedded_asset(file_rel, JS_SHIM) {
                    let ct_header = Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap();
                    let cors = Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap();
                    let response = Response::from_data(data).with_header(ct_header).with_header(cors);
                    let _ = req.respond(response);
                } else {
                    let _ = req.respond(Response::from_string("404 Bulunamadı").with_status_code(StatusCode(404)));
                }
            }
        });
    }

    Ok(port)
}

fn resolve_webui_dir() -> PathBuf {
    // 1. Mevcut çalışma dizinindeki webui (geliştirme modunda cargo run için)
    if let Ok(cwd) = std::env::current_dir() {
        let candidate = cwd.join("webui");
        if candidate.is_dir() {
            return candidate;
        }
    }

    // 2. macOS .app bundle içindeki Resources/webui kontrolü
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let bundle_res = parent.join("../Resources/webui");
            if bundle_res.is_dir() {
                if let Ok(canonical) = bundle_res.canonicalize() {
                    return canonical;
                }
                return bundle_res;
            }

            // Exe ile aynı klasördeki webui
            let sibling = parent.join("webui");
            if sibling.is_dir() {
                return sibling;
            }
        }
    }

    PathBuf::from("webui")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "windows")]
    portable_dlls::setup_embedded_dlls();

    let args = CliArgs::parse();

    // Komut satırı (CLI) modu
    if args.once {
        println!("Webcam Capture (Rust CLI modu)");
        let mut state = AppState::new();
        let res = state.start_camera();
        if res.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            eprintln!("Kamera başlatılamadı: {:?}", res.get("message"));
            std::process::exit(1);
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        let cap_res = state.capture_now(&args.name);
        println!("{}", serde_json::to_string_pretty(&cap_res)?);
        state.stop_camera();
        return Ok(());
    }

    let shared_state = Arc::new(Mutex::new(AppState::new()));
    let webui_dir = resolve_webui_dir();

    // Tao Olay Döngüsü (Özel UserEvent desteği ile)
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let event_proxy = event_loop.create_proxy();

    let port = start_http_server(Arc::clone(&shared_state), webui_dir, event_proxy)?;
    let app_url = format!("http://127.0.0.1:{}/", port);

    if args.server_only {
        println!("Webcam Capture sunucusu çalışıyor: {}", app_url);
        println!("Çıkmak için CTRL+C tuşlayın.");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }

    let app_icon = load_app_icon();

    // Ana Masaüstü Webview Penceresi (Tao + Wry)
    let mut main_builder = WindowBuilder::new()
        .with_title("Webcam Capture")
        .with_inner_size(LogicalSize::new(1180.0, 780.0))
        .with_min_inner_size(LogicalSize::new(960.0, 620.0));

    if let Some(ref icon) = app_icon {
        main_builder = main_builder.with_window_icon(Some(icon.clone()));
    }

    let main_window = main_builder.build(&event_loop)?;
    let main_window_id = main_window.id();

    let _main_webview = WebViewBuilder::new()
        .with_url(&app_url)
        .build(&main_window)?;

    println!("Webcam Capture (Rust) masaüstü arayüzü başlatıldı: {}", app_url);

    let state_for_events = Arc::clone(&shared_state);
    let mut target_window_holder: Option<(Window, WebView)> = None;
    let target_icon = app_icon;

    event_loop.run(move |event, event_loop_target, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(StartCause::Init) => {}
            Event::UserEvent(UserEvent::OpenTargetWindow) => {
                if let Some((win, _)) = &target_window_holder {
                    win.set_focus();
                    win.set_minimized(false);
                } else {
                    let mut target_win_builder = WindowBuilder::new()
                        .with_title("Hedef Görüntü — İkinci Monitör")
                        .with_inner_size(LogicalSize::new(480.0, 640.0))
                        .with_min_inner_size(LogicalSize::new(320.0, 420.0));

                    if let Some(ref icon) = target_icon {
                        target_win_builder = target_win_builder.with_window_icon(Some(icon.clone()));
                    }

                    if let Ok(win) = target_win_builder.build(event_loop_target) {
                        let target_url = format!("http://127.0.0.1:{}/target_monitor.html", port);
                        if let Ok(wv) = WebViewBuilder::new().with_url(&target_url).build(&win) {
                            target_window_holder = Some((win, wv));
                        }
                    }
                }
            }
            Event::UserEvent(UserEvent::CloseTargetWindow) => {
                target_window_holder = None;
                let mut state = state_for_events.lock().unwrap();
                state.target_window_open = false;
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if let Some((win, _)) = &target_window_holder {
                    if win.id() == window_id {
                        target_window_holder = None;
                        let mut state = state_for_events.lock().unwrap();
                        state.target_window_open = false;
                        return;
                    }
                }

                if window_id == main_window_id {
                    if let Ok(mut state) = state_for_events.lock() {
                        let _ = state.stop_camera();
                    }
                    *control_flow = ControlFlow::Exit;
                    std::process::exit(0);
                }
            }
            Event::LoopDestroyed => {
                std::process::exit(0);
            }
            _ => (),
        }
    });
}
