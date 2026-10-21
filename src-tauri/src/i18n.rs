//! Backend strings in English and Turkish. Add a message: one row in `MSGS` with the same
//! `{placeholders}` in both texts (a test checks). The UI has its own dictionary in src/i18n.js.

use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    En,
    Tr,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Tr => "tr",
        }
    }

    /// Anything but "tr" is English.
    pub fn from_code(s: &str) -> Lang {
        if s == "tr" {
            Lang::Tr
        } else {
            Lang::En
        }
    }
}

impl Serialize for Lang {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.code())
    }
}

impl<'de> Deserialize<'de> for Lang {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Lang, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        Ok(v.as_str().map_or(Lang::En, Lang::from_code))
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Language of the saved settings; set at startup and on every settings change.
pub fn lang() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == 1 {
        Lang::Tr
    } else {
        Lang::En
    }
}

pub fn set_lang(l: Lang) {
    CURRENT.store(u8::from(l == Lang::Tr), Ordering::Relaxed);
}

/// (key, English, Turkish)
pub const MSGS: &[(&str, &str, &str)] = &[
    // context menu
    ("menu.convert_to", "Convert to {fmt}", "{fmt} olarak dönüştür"),
    // tray
    ("tray.open", "Settings", "Ayarlar"),
    ("tray.quit", "Quit", "Çıkış"),
    // notifications
    ("notify.dl_title", "Downloading ffmpeg", "ffmpeg indiriliyor"),
    (
        "notify.dl_body",
        "Video and MP3/OGG conversion needs ffmpeg, a free open-source tool. It will be downloaded once (about 170 MB).",
        "Video ve MP3/OGG dönüştürme için ücretsiz, açık kaynaklı ffmpeg aracı bir kez indirilecek (yaklaşık 170 MB).",
    ),
    ("notify.ready_title", "ffmpeg ready", "ffmpeg hazır"),
    (
        "notify.ready_body",
        "Video and audio conversion is ready to use.",
        "Video ve ses dönüştürme kullanıma hazır.",
    ),
    ("notify.done_title", "Converted", "Dönüştürüldü"),
    ("notify.failed_title", "Conversion failed", "Dönüştürme başarısız"),
    (
        "notify.target_off",
        "{fmt} is turned off in settings",
        "{fmt} biçimi ayarlardan kapatılmış",
    ),
    // commands
    (
        "cmd.autostart",
        "Could not change the startup setting: {err}",
        "Otomatik başlatma ayarlanamadı: {err}",
    ),
    (
        "cmd.menu_save",
        "Could not save the context menu: {err}",
        "Sağ tık menüsü kaydedilemedi: {err}",
    ),
    (
        "cmd.menu_remove",
        "Could not remove the context menu: {err}",
        "Sağ tık menüsü kaldırılamadı: {err}",
    ),
    (
        "cmd.save",
        "Could not save settings: {err}",
        "Ayarlar kaydedilemedi: {err}",
    ),
    ("cmd.bad_corner", "Invalid corner", "Geçersiz köşe"),
    ("cmd.bad_seconds", "Invalid duration", "Geçersiz süre"),
    ("cmd.bad_path", "Invalid path", "Geçersiz yol"),
    ("cmd.no_path", "Path not found", "Yol bulunamadı"),
    // jobs and panel
    ("job.done", "Done", "Tamamlandı"),
    ("job.cancelled", "Cancelled", "İptal edildi"),
    (
        "job.needs_ffmpeg",
        "This conversion needs ffmpeg, which is not ready yet",
        "Bu dönüşüm için ffmpeg gerekiyor, henüz hazır değil",
    ),
    ("panel.demo_name", "sample-photo.png", "ornek-foto.png"),
    // name template validation
    (
        "tpl.length",
        "The file name must be 1 to {max} characters",
        "Ad şablonu 1 ile {max} karakter arasında olmalı",
    ),
    (
        "tpl.unknown_token",
        "The file name has an unknown tag; only {name}, {format} and {date} are allowed",
        "Ad şablonunda bilinmeyen simge var; yalnızca {name}, {format} ve {date} kullanılabilir",
    ),
    (
        "tpl.bad_char",
        "The file name contains an invalid character: {c}",
        "Ad şablonunda geçersiz karakter var: {c}",
    ),
    (
        "tpl.trailing",
        "The file name cannot end with a dot or a space",
        "Ad şablonu nokta veya boşlukla bitemez",
    ),
    (
        "tpl.reserved",
        "The file name would be a reserved name in Windows",
        "Ad şablonu Windows'ta ayrılmış bir ad üretiyor",
    ),
    // conversion errors
    (
        "ce.invalid_path",
        "Invalid file path: {detail}",
        "Geçersiz dosya yolu: {detail}",
    ),
    ("ce.not_found", "File not found", "Dosya bulunamadı"),
    ("ce.access_denied", "Access denied", "Erişim reddedildi"),
    ("ce.disk_full", "Disk full", "Disk dolu"),
    (
        "ce.corrupt",
        "The file is corrupt or unreadable",
        "Dosya bozuk veya okunamıyor",
    ),
    ("ce.unsupported", "Unsupported format", "Desteklenmeyen biçim"),
    ("ce.too_large", "File too large", "Dosya çok büyük"),
    (
        "ce.target_size",
        "The size limit cannot be reached without making the result unusable",
        "Boyut sınırına, sonuç kullanılamaz hale gelmeden ulaşılamıyor",
    ),
    (
        "ce.protected",
        "The file is password protected",
        "Dosya parola korumalı",
    ),
    (
        "ce.needs_ffmpeg",
        "This conversion needs ffmpeg",
        "Bu dönüşüm için ffmpeg gerekiyor",
    ),
    ("ce.cancelled", "Cancelled", "İptal edildi"),
    (
        "ce.encode",
        "Conversion failed: {detail}",
        "Dönüştürme başarısız: {detail}",
    ),
    ("ce.io", "File error: {detail}", "Dosya hatası: {detail}"),
    ("ce.not_file", "not a file", "dosya değil"),
    ("ce.not_dir", "not a folder", "klasör değil"),
    (
        "ce.network_path",
        "network or device path",
        "ağ veya aygıt yolu",
    ),
    ("ce.bad_name", "invalid file name", "dosya adı geçersiz"),
    ("ce.no_free_name", "no free file name", "uygun ad bulunamadı"),
    (
        "ce.no_stream",
        "The file has no stream needed for this conversion",
        "Dosyada bu dönüşüm için gereken akış yok",
    ),
    ("ce.empty_output", "Empty output", "Boş çıktı"),
    ("ce.unknown", "unknown error", "bilinmeyen hata"),
    (
        "ce.ffmpeg_start",
        "Could not start ffmpeg: {detail}",
        "ffmpeg başlatılamadı: {detail}",
    ),
    (
        "ce.ffmpeg_wait",
        "Could not wait for ffmpeg: {detail}",
        "ffmpeg beklenemedi: {detail}",
    ),
    (
        "ce.recycle",
        "Could not delete the original file: {detail}",
        "Orijinal dosya silinemedi: {detail}",
    ),
    // ffmpeg download errors
    (
        "ff.read",
        "Could not read the file: {detail}",
        "Dosya okunamadı: {detail}",
    ),
    (
        "ff.hash",
        "The downloaded file could not be verified (SHA-256 mismatch)",
        "İndirilen dosya doğrulanamadı (SHA-256 uyuşmuyor)",
    ),
    (
        "ff.start",
        "Could not start the download: {detail}",
        "İndirme başlatılamadı: {detail}",
    ),
    (
        "ff.too_big",
        "The file to download is larger than expected",
        "İndirilecek dosya beklenenden büyük",
    ),
    (
        "ff.tmp",
        "Could not create a temporary file: {detail}",
        "Geçici dosya oluşturulamadı: {detail}",
    ),
    (
        "ff.cut",
        "The download was interrupted: {detail}",
        "İndirme kesildi: {detail}",
    ),
    (
        "ff.write",
        "Could not write to disk: {detail}",
        "Diske yazılamadı: {detail}",
    ),
    (
        "ff.archive",
        "Could not open the archive: {detail}",
        "Arşiv açılamadı: {detail}",
    ),
    (
        "ff.unsafe",
        "The archive contains an unsafe file path",
        "Arşivde güvensiz dosya yolu var",
    ),
    (
        "ff.no_exe",
        "ffmpeg.exe was not found in the archive",
        "Arşivde ffmpeg.exe bulunamadı",
    ),
    (
        "ff.exe_big",
        "ffmpeg.exe is larger than expected",
        "ffmpeg.exe beklenenden büyük",
    ),
    (
        "ff.mkdir",
        "Could not create the folder: {detail}",
        "Klasör oluşturulamadı: {detail}",
    ),
    (
        "ff.run",
        "ffmpeg could not be run",
        "ffmpeg çalıştırılamadı",
    ),
];

/// Text for `key`; an unknown key comes back as itself (a test catches those).
pub fn t(lang: Lang, key: &str) -> &str {
    MSGS.iter()
        .find(|m| m.0 == key)
        .map_or(key, |m| match lang {
            Lang::En => m.1,
            Lang::Tr => m.2,
        })
}

/// Like `t`, with each `{name}` replaced from `vars`.
pub fn fmt(lang: Lang, key: &str, vars: &[(&str, &str)]) -> String {
    let mut s = t(lang, key).to_string();
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholders(s: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = s;
        while let Some(i) = rest.find('{') {
            let Some(j) = rest[i..].find('}') else { break };
            out.push(rest[i + 1..i + j].to_string());
            rest = &rest[i + j + 1..];
        }
        out.sort();
        out
    }

    #[test]
    fn every_key_is_unique_and_filled_in_both_languages() {
        let mut seen = std::collections::HashSet::new();
        for (k, en, tr) in MSGS {
            assert!(seen.insert(*k), "duplicate key {k}");
            assert!(!en.is_empty() && !tr.is_empty(), "{k}");
        }
    }

    #[test]
    fn placeholders_match_between_languages() {
        for (k, en, tr) in MSGS {
            assert_eq!(placeholders(en), placeholders(tr), "{k}");
        }
    }

    #[test]
    fn lookup_and_format() {
        assert_eq!(t(Lang::En, "tray.quit"), "Quit");
        assert_eq!(t(Lang::Tr, "tray.quit"), "Çıkış");
        assert_eq!(t(Lang::En, "no.such.key"), "no.such.key");
        let v = [("fmt", "JPG")];
        assert_eq!(fmt(Lang::En, "menu.convert_to", &v), "Convert to JPG");
        assert_eq!(fmt(Lang::Tr, "menu.convert_to", &v), "JPG olarak dönüştür");
    }

    #[test]
    fn language_defaults_to_english() {
        assert_eq!(Lang::default(), Lang::En);
        assert_eq!(Lang::from_code("tr"), Lang::Tr);
        for bad in ["", "de", "TR", "english"] {
            assert_eq!(Lang::from_code(bad), Lang::En, "{bad}");
        }
        assert_eq!(serde_json::from_str::<Lang>("\"tr\"").unwrap(), Lang::Tr);
        assert_eq!(serde_json::from_str::<Lang>("\"fr\"").unwrap(), Lang::En);
        assert_eq!(serde_json::from_str::<Lang>("7").unwrap(), Lang::En);
        assert_eq!(serde_json::to_string(&Lang::Tr).unwrap(), "\"tr\"");
    }
}
