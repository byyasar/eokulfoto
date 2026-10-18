use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub camera_index: u32,
    pub cam_resolution: String,
    pub width: u32,
    pub height: u32,
    pub image_format: String,
    pub jpeg_quality: u8,
    pub edevlet_limit: bool,
    pub output_dir: PathBuf,
    pub face_detect: bool,
    pub face_head_ratio: f32,
    pub face_v_offset: f32,
    pub face_smoothing: f32,
    pub face_detect_every: u32,
    pub frame_zoom: f32,
}

fn default_output_dir() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if cwd != Path::new("/") {
            return PathBuf::from("captures");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join("Pictures").join("WebcamCapture");
    }
    PathBuf::from("captures")
}

impl Default for Config {
    fn default() -> Self {
        Self {
            camera_index: 0,
            cam_resolution: "auto".to_string(),
            width: 133,
            height: 171,
            image_format: "png".to_string(),
            jpeg_quality: 95,
            edevlet_limit: true,
            output_dir: default_output_dir(),
            face_detect: true,
            face_head_ratio: 0.65,
            face_v_offset: 0.10,
            face_smoothing: 0.35,
            face_detect_every: 3,
            frame_zoom: 1.0,
        }
    }
}

impl Config {
    pub fn get_env_path() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                let candidate = parent.join(".env");
                if candidate.exists() {
                    return candidate;
                }
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            let candidate = cwd.join(".env");
            if candidate.exists() {
                return candidate;
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                return parent.join(".env");
            }
        }
        PathBuf::from(".env")
    }

    pub fn load() -> Self {
        let env_path = Self::get_env_path();
        let _ = dotenvy::from_path(&env_path);
        let mut cfg = Config::default();

        if let Ok(val) = std::env::var("CAMERA_INDEX") {
            if let Ok(v) = val.trim().parse() {
                cfg.camera_index = v;
            }
        }
        if let Ok(val) = std::env::var("CAM_RESOLUTION") {
            cfg.cam_resolution = val.trim().to_lowercase();
        }
        if let Ok(val) = std::env::var("TARGET_WIDTH") {
            if let Ok(v) = val.trim().parse() {
                cfg.width = v;
            }
        }
        if let Ok(val) = std::env::var("TARGET_HEIGHT") {
            if let Ok(v) = val.trim().parse() {
                cfg.height = v;
            }
        }
        if let Ok(val) = std::env::var("IMAGE_FORMAT") {
            let fmt = val.trim().to_lowercase();
            if fmt == "jpg" || fmt == "jpeg" || fmt == "png" {
                cfg.image_format = if fmt == "jpeg" { "jpg".to_string() } else { fmt };
            }
        }
        if let Ok(val) = std::env::var("JPEG_QUALITY") {
            if let Ok(v) = val.trim().parse() {
                cfg.jpeg_quality = v;
            }
        }
        if let Ok(val) = std::env::var("EDEVLET_LIMIT") {
            let v = val.trim().to_lowercase();
            cfg.edevlet_limit = matches!(v.as_str(), "1" | "true" | "yes" | "on" | "evet");
        }
        if let Ok(val) = std::env::var("OUTPUT_DIR") {
            let clean = val.trim();
            if !clean.is_empty() {
                cfg.output_dir = PathBuf::from(clean);
            }
        }
        if let Ok(val) = std::env::var("FACE_DETECT") {
            let v = val.trim().to_lowercase();
            cfg.face_detect = matches!(v.as_str(), "1" | "true" | "yes" | "on" | "evet");
        }
        if let Ok(val) = std::env::var("FACE_HEAD_RATIO") {
            if let Ok(v) = val.trim().parse() {
                cfg.face_head_ratio = v;
            }
        }
        if let Ok(val) = std::env::var("FACE_V_OFFSET") {
            if let Ok(v) = val.trim().parse() {
                cfg.face_v_offset = v;
            }
        }
        if let Ok(val) = std::env::var("FACE_SMOOTHING") {
            if let Ok(v) = val.trim().parse() {
                cfg.face_smoothing = v;
            }
        }
        if let Ok(val) = std::env::var("FACE_DETECT_EVERY") {
            if let Ok(v) = val.trim().parse() {
                cfg.face_detect_every = v;
            }
        }
        if let Ok(val) = std::env::var("FRAME_ZOOM") {
            if let Ok(v) = val.trim().parse() {
                cfg.frame_zoom = v;
            }
        }

        cfg
    }

    pub fn get_effective_output_dir(&self) -> PathBuf {
        if self.output_dir.is_absolute() {
            self.output_dir.clone()
        } else if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                parent.join(&self.output_dir)
            } else if let Ok(cwd) = std::env::current_dir() {
                cwd.join(&self.output_dir)
            } else {
                self.output_dir.clone()
            }
        } else if let Ok(cwd) = std::env::current_dir() {
            cwd.join(&self.output_dir)
        } else {
            self.output_dir.clone()
        }
    }

    pub fn update_env_file(&self, path: &Path) -> std::io::Result<()> {
        let mut map = std::collections::BTreeMap::new();
        map.insert("CAMERA_INDEX", self.camera_index.to_string());
        map.insert("CAM_RESOLUTION", self.cam_resolution.clone());
        map.insert("TARGET_WIDTH", self.width.to_string());
        map.insert("TARGET_HEIGHT", self.height.to_string());
        map.insert("IMAGE_FORMAT", self.image_format.clone());
        map.insert("JPEG_QUALITY", self.jpeg_quality.to_string());
        map.insert("EDEVLET_LIMIT", self.edevlet_limit.to_string());
        map.insert("OUTPUT_DIR", self.output_dir.to_string_lossy().to_string());
        map.insert("FACE_DETECT", self.face_detect.to_string());
        map.insert("FACE_HEAD_RATIO", format!("{:.2}", self.face_head_ratio));
        map.insert("FACE_V_OFFSET", format!("{:.2}", self.face_v_offset));
        map.insert("FACE_SMOOTHING", format!("{:.2}", self.face_smoothing));
        map.insert("FACE_DETECT_EVERY", self.face_detect_every.to_string());
        map.insert("FRAME_ZOOM", format!("{:.2}", self.frame_zoom));

        let content = if path.exists() {
            fs::read_to_string(path).unwrap_or_default()
        } else {
            String::new()
        };

        let mut lines: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for line in content.lines() {
            let stripped = line.trim();
            if !stripped.is_empty() && !stripped.starts_with('#') && stripped.contains('=') {
                if let Some((k, _)) = stripped.split_once('=') {
                    let key = k.trim();
                    if let Some(val) = map.get(key) {
                        lines.push(format!("{}={}", key, val));
                        seen.insert(key.to_string());
                        continue;
                    }
                }
            }
            lines.push(line.to_string());
        }

        for (k, v) in map {
            if !seen.contains(k) {
                lines.push(format!("{}={}", k, v));
            }
        }

        fs::write(path, lines.join("\n") + "\n")
    }
}
