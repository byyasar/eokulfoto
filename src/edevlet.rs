use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use chrono::Local;
use image::{DynamicImage, ImageFormat, RgbImage};

pub const MIN_FILE_BYTES: usize = 20 * 1024;
pub const MAX_FILE_BYTES: usize = 150 * 1024;
pub const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

pub fn sanitize_filename(name: &str) -> String {
    let mut clean = name.trim().to_string();
    if clean.is_empty() {
        return clean;
    }
    // Bilinen uzantıları temizle (.png, .jpg, .jpeg)
    let lower = clean.to_lowercase();
    if lower.ends_with(".png") {
        clean.truncate(clean.len() - 4);
    } else if lower.ends_with(".jpg") {
        clean.truncate(clean.len() - 4);
    } else if lower.ends_with(".jpeg") {
        clean.truncate(clean.len() - 5);
    }
    // Geçersiz dosya sistemi karakterlerini temizle
    let filtered: String = clean
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 32 => '_',
            c => c,
        })
        .collect();

    // Çoklu alt çizgileri tek alt çizgiye indir
    let mut dedup = String::new();
    let mut prev_underscore = false;
    for c in filtered.chars() {
        if c == '_' {
            if !prev_underscore {
                dedup.push(c);
                prev_underscore = true;
            }
        } else {
            dedup.push(c);
            prev_underscore = false;
        }
    }
    dedup.trim_matches(|c| c == '.' || c == ' ' || c == '_').to_string()
}

pub fn encode_png_with_phys(img: &RgbImage) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    let dyn_img = DynamicImage::ImageRgb8(img.clone());
    dyn_img
        .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .map_err(|e| format!("PNG kodlama hatasi: {}", e))?;

    // PNG pHYs parçası ekle (3800 piksel/metre = 35x45mm ebadı)
    // pHYs formatı: 4 bayt x, 4 bayt y, 1 bayt unit (1 = meter)
    // 3800 = 0x00000ED8
    if buf.len() > 33 && &buf[..8] == PNG_MAGIC {
        let mut phys_payload = Vec::new();
        phys_payload.extend_from_slice(&3800u32.to_be_bytes()); // X axis
        phys_payload.extend_from_slice(&3800u32.to_be_bytes()); // Y axis
        phys_payload.push(1u8); // unit: meter

        let mut phys_chunk = Vec::new();
        phys_chunk.extend_from_slice(&(phys_payload.len() as u32).to_be_bytes());
        phys_chunk.extend_from_slice(b"pHYs");
        phys_chunk.extend_from_slice(&phys_payload);

        let mut crc_data = Vec::new();
        crc_data.extend_from_slice(b"pHYs");
        crc_data.extend_from_slice(&phys_payload);
        let crc = crc32fast::hash(&crc_data);
        phys_chunk.extend_from_slice(&crc.to_be_bytes());

        // IHDR parçası 8 bayt imza + 4 len + 4 'IHDR' + 13 veri + 4 CRC = 33 bayt
        let ihdr_end = 33;
        let mut new_buf = Vec::with_capacity(buf.len() + phys_chunk.len());
        new_buf.extend_from_slice(&buf[..ihdr_end]);
        new_buf.extend_from_slice(&phys_chunk);
        new_buf.extend_from_slice(&buf[ihdr_end..]);
        return Ok(new_buf);
    }

    Ok(buf)
}

pub fn pad_png(mut data: Vec<u8>) -> Vec<u8> {
    if data.len() >= MIN_FILE_BYTES {
        return data;
    }
    let need = MIN_FILE_BYTES - data.len();
    // Parça genel gideri: len(4) + "tEXt"(4) + CRC(4) + "Comment\0"(8) = 20 bayt
    let filler_len = if need > 20 { need - 20 } else { 1 };
    let mut payload = Vec::new();
    payload.extend_from_slice(b"Comment\0");
    payload.resize(payload.len() + filler_len, b' ');

    let mut chunk = Vec::new();
    chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    chunk.extend_from_slice(b"tEXt");
    chunk.extend_from_slice(&payload);

    let mut crc_data = Vec::new();
    crc_data.extend_from_slice(b"tEXt");
    crc_data.extend_from_slice(&payload);
    let crc = crc32fast::hash(&crc_data);
    chunk.extend_from_slice(&crc.to_be_bytes());

    let insert_pos = 33; // IHDR sonrası
    if data.len() >= insert_pos {
        let mut out = Vec::with_capacity(data.len() + chunk.len());
        out.extend_from_slice(&data[..insert_pos]);
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&data[insert_pos..]);
        out
    } else {
        data.extend_from_slice(&chunk);
        data
    }
}

pub fn encode_jpeg(img: &RgbImage, quality: u8) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
    encoder
        .encode_image(&DynamicImage::ImageRgb8(img.clone()))
        .map_err(|e| format!("JPEG kodlama hatasi: {}", e))?;

    // DPI bilgisini JFIF APP0'a göm (97 DPI = ~35x45 mm)
    if buf.len() > 18 && buf[0] == 0xFF && buf[1] == 0xD8 && buf[2] == 0xFF && buf[3] == 0xE0 {
        buf[13] = 1; // density units: 1 = dots per inch
        buf[14] = 0;
        buf[15] = 97; // Xdensity: 97 DPI
        buf[16] = 0;
        buf[17] = 97; // Ydensity: 97 DPI
    }

    Ok(buf)
}

pub fn pad_jpeg(mut data: Vec<u8>) -> Vec<u8> {
    while data.len() < MIN_FILE_BYTES {
        let diff = MIN_FILE_BYTES - data.len();
        let room = diff.min(65531);
        let mut seg = Vec::new();
        seg.push(0xFF);
        seg.push(0xFE); // COM marker
        let seg_len = (room + 2) as u16;
        seg.extend_from_slice(&seg_len.to_be_bytes());
        seg.resize(seg.len() + room, b' ');

        // JFIF APP0 segmentinden hemen sonra yerleştir
        let insert_pos = if data.len() > 6 && data[2] == 0xFF && data[3] == 0xE0 {
            let app0_len = u16::from_be_bytes([data[4], data[5]]) as usize;
            4 + app0_len
        } else {
            2
        };

        if insert_pos <= data.len() {
            let mut out = Vec::with_capacity(data.len() + seg.len());
            out.extend_from_slice(&data[..insert_pos]);
            out.extend_from_slice(&seg);
            out.extend_from_slice(&data[insert_pos..]);
            data = out;
        } else {
            data.extend_from_slice(&seg);
        }
    }
    data
}

pub fn encode_sized(
    img: &RgbImage,
    fmt: &str,
    quality: u8,
    edevlet_limit: bool,
) -> Result<(Vec<u8>, &'static str), String> {
    let clean_fmt = fmt.trim().to_lowercase();
    let is_png_req = clean_fmt == "png";

    if !edevlet_limit {
        if is_png_req {
            let data = encode_png_with_phys(img)?;
            return Ok((data, "png"));
        } else {
            let data = encode_jpeg(img, quality)?;
            return Ok((data, "jpg"));
        }
    }

    // e-Devlet boyutu kısıtı (20 - 150 KB)
    if is_png_req {
        let png = encode_png_with_phys(img)?;
        if png.len() >= MIN_FILE_BYTES && png.len() <= MAX_FILE_BYTES {
            return Ok((png, "png"));
        }
        if png.len() < MIN_FILE_BYTES {
            return Ok((pad_png(png), "png"));
        }
        // PNG 150 KB'yi aşarsa JPEG merdivenine düş
    }

    let ladder = [
        100, 99, 98, 97, 96, 95, 93, 90, 88, 85, 80, 75, 70, 65, 60, 50, 40, 30, 20, 10,
    ];
    let start_quality = quality.clamp(10, 100);
    let mut selected_bytes = Vec::new();

    for &q in &ladder {
        if q > start_quality {
            continue;
        }
        let bytes = encode_jpeg(img, q)?;
        if bytes.len() <= MAX_FILE_BYTES {
            selected_bytes = bytes;
            break;
        }
    }

    if selected_bytes.is_empty() {
        selected_bytes = encode_jpeg(img, 10)?;
    }

    if selected_bytes.len() < MIN_FILE_BYTES {
        selected_bytes = pad_jpeg(selected_bytes);
    }

    Ok((selected_bytes, "jpg"))
}

pub fn save_capture(
    img: &RgbImage,
    output_dir: &Path,
    custom_name: &str,
    fmt: &str,
    quality: u8,
    edevlet_limit: bool,
) -> Result<PathBuf, String> {
    fs::create_dir_all(output_dir).map_err(|e| format!("Klasor olusturulamadi: {}", e))?;

    let (bytes, ext) = encode_sized(img, fmt, quality, edevlet_limit)?;
    let clean = sanitize_filename(custom_name);

    let target_path = if !clean.is_empty() {
        let mut candidate = output_dir.join(format!("{}.{}", clean, ext));
        let mut counter = 1;
        while candidate.exists() {
            candidate = output_dir.join(format!("{}_{}.{}", clean, counter, ext));
            counter += 1;
        }
        candidate
    } else {
        let stamp = Local::now().format("%Y%m%d_%H%M%S_%3f").to_string();
        output_dir.join(format!("capture_{}.{}", stamp, ext))
    };

    fs::write(&target_path, &bytes).map_err(|e| format!("Dosya kaydedilemedi: {}", e))?;
    Ok(target_path)
}
