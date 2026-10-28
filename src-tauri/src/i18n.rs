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

/// Windows display language: Turkish for a Turkish UI, English for every other language.
pub fn system_lang() -> Lang {
    // SAFETY: a plain query with no arguments.
    let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    if id & 0x3ff == 0x1f {
        Lang::Tr
    } else {
        Lang::En
    }
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
    // merge verbs
    ("menu.merge_pdf", "Merge into PDF", "PDF olarak birleştir"),
    (
        "menu.merge_audio",
        "Merge audio into one file",
        "Sesleri tek dosyada birleştir",
    ),
    // built-in presets
    ("preset.web", "Shrink for web", "Web için küçült"),
    ("preset.whatsapp", "For WhatsApp", "WhatsApp için"),
    ("preset.email", "Email: under 10 MB", "E-posta: 10 MB altı"),
    ("preset.discord", "Discord: under 10 MB", "Discord: 10 MB altı"),
    (
        "preset.strip",
        "Remove location & camera info",
        "Konum ve kamera bilgisini sil",
    ),
    ("preset.pdf", "Make a PDF", "PDF yap"),
    ("preset.webp", "Make a WebP", "WebP yap"),
    ("preset.ico", "Make an icon (ICO)", "Simge yap (ICO)"),
    ("preset.instagram", "For Instagram (1080 px)", "Instagram için (1080 px)"),
    ("preset.podcast", "Podcast: mono MP3, 96 kbps", "Podcast: mono MP3, 96 kbps"),
    // preset errors
    (
        "preset.bad_name",
        "The preset name must be 1 to 40 characters",
        "Hazır ayar adı 1 ile 40 karakter arasında olmalı",
    ),
    (
        "preset.bad_id",
        "The preset id may only use a-z, 0-9, - and _ (up to 32 characters)",
        "Hazır ayar kimliği yalnızca a-z, 0-9, - ve _ içerebilir (en çok 32 karakter)",
    ),
    (
        "preset.builtin",
        "Built-in presets cannot be changed",
        "Yerleşik hazır ayarlar değiştirilemez",
    ),
    (
        "preset.bad_trim",
        "The cut must end after it starts",
        "Kesim bitişi başlangıçtan sonra olmalı",
    ),
    (
        "preset.bad_kinds",
        "Pick at least one file type for the preset",
        "Hazır ayar için en az bir dosya türü seçin",
    ),
    (
        "preset.bad_target",
        "The preset has an unknown target format",
        "Hazır ayarın hedef biçimi bilinmiyor",
    ),
    (
        "preset.same_needs_option",
        "Keeping the format needs a resize, a size limit or removing location and camera info",
        "Biçimi korumak için boyutlandırma, boyut sınırı ya da konum ve kamera bilgisini silme gerekir",
    ),
    (
        "preset.too_many",
        "You can have at most {max} custom presets",
        "En fazla {max} özel hazır ayar olabilir",
    ),
    ("preset.not_found", "Preset not found", "Hazır ayar bulunamadı"),
    (
        "notify.preset_skip",
        "{n} file(s) skipped: not supported by {name}",
        "{n} dosya atlandı: {name} bunları desteklemiyor",
    ),
    (
        "job.merge_name",
        "Merging {n} files",
        "{n} dosya birleştiriliyor",
    ),
    // undo
    (
        "undo.not_found",
        "That history entry no longer exists",
        "Bu geçmiş kaydı artık yok",
    ),
    (
        "undo.nothing",
        "There is nothing to undo for this entry",
        "Bu kayıt için geri alınacak bir şey yok",
    ),
    (
        "undo.no_output",
        "The converted file is already gone",
        "Dönüştürülen dosya zaten yok",
    ),
    (
        "undo.changed",
        "The converted file was changed after it was made, so it was left alone",
        "Dönüştürülen dosya sonradan değişmiş, bu yüzden dokunulmadı",
    ),
    (
        "undo.occupied",
        "A file already exists at the original location: {path}",
        "Özgün konumda zaten bir dosya var: {path}",
    ),
    (
        "undo.restore_failed",
        "Could not restore the original ({detail}). It is still in the Recycle Bin: {path}",
        "Orijinal dosya geri getirilemedi ({detail}). Hâlâ Geri Dönüşüm Kutusu'nda: {path}",
    ),
    (
        "undo.delete_failed",
        "Could not move the converted file to the Recycle Bin: {detail}",
        "Dönüştürülen dosya Geri Dönüşüm Kutusu'na taşınamadı: {detail}",
    ),
    // update check
    (
        "update.available",
        "Version {latest} is available (you have {current})",
        "{latest} sürümü yayında (sizdeki: {current})",
    ),
    (
        "update.up_to_date",
        "You are up to date (version {current})",
        "Güncelsiniz (sürüm {current})",
    ),
    (
        "update.none_yet",
        "No releases have been published yet, so you are up to date",
        "Henüz sürüm yayınlanmamış, yani güncelsiniz",
    ),
    (
        "update.failed",
        "Could not check for updates: {err}",
        "Güncellemeler denetlenemedi: {err}",
    ),
    (
        "update.not_installed",
        "Updating works from the installed app only. Download the new version from the release page.",
        "Güncelleme sadece kurulu uygulamada çalışır. Yeni sürümü sürüm sayfasından indir.",
    ),
    (
        "update.no_asset",
        "This release has no installer to download",
        "Bu sürümde indirilecek bir kurulum dosyası yok",
    ),
    (
        "update.bad_response",
        "The update server sent an unexpected answer",
        "Güncelleme sunucusu beklenmeyen bir yanıt verdi",
    ),
    (
        "update.bad_url",
        "This link cannot be opened",
        "Bu bağlantı açılamaz",
    ),
    (
        "update.open_failed",
        "Could not open the browser",
        "Tarayıcı açılamadı",
    ),
    ("update.title", "Update available", "Güncelleme var"),
    // folder watch
    (
        "watch.too_many",
        "You can have at most {max} watch rules",
        "En fazla {max} izleme kuralı olabilir",
    ),
    (
        "watch.bad_id",
        "A watch rule has an invalid or duplicate id",
        "Bir izleme kuralının kimliği geçersiz ya da tekrarlanıyor",
    ),
    (
        "watch.bad_folder",
        "Pick a folder on a local drive: {folder}",
        "Yerel bir sürücüdeki klasörü seçin: {folder}",
    ),
    (
        "watch.forbidden_folder",
        "This folder cannot be used for watching or output: {folder}",
        "Bu klasör izleme veya çıktı için kullanılamaz: {folder}",
    ),
    (
        "watch.bad_ext",
        "Unknown file type in a watch rule: {ext}",
        "İzleme kuralında bilinmeyen dosya türü: {ext}",
    ),
    (
        "watch.bad_output",
        "A watch rule has an invalid output location",
        "Bir izleme kuralının çıktı konumu geçersiz",
    ),
    (
        "watch.missing_folder",
        "The folder does not exist: {folder}",
        "Klasör bulunamadı: {folder}",
    ),
    (
        "watch.start_failed",
        "Could not watch the folder: {err}",
        "Klasör izlenemedi: {err}",
    ),
    ("watch.label", "Watch: {folder}", "İzleme: {folder}"),
    // command line
    (
        "cli.no_ffmpeg",
        "This conversion needs ffmpeg, which is not installed. Start the app once so it can install it.",
        "Bu dönüşüm için ffmpeg gerekiyor ama kurulu değil. Kurması için uygulamayı bir kez başlatın.",
    ),
    (
        "cli.no_office",
        "Office documents need document support, which is not downloaded. Get it in the app under Settings > Advanced, or run `tessera --install-office`.",
        "Ofis belgeleri için belge desteği gerekiyor ama indirilmemiş. Uygulamada Ayarlar > Gelişmiş bölümünden indirin ya da `tessera --install-office` çalıştırın.",
    ),
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
    // folder verbs
    (
        "menu.folder_images_to",
        "Convert all images to {fmt}",
        "Tüm resimleri {fmt} olarak dönüştür",
    ),
    (
        "menu.folder_audio_to",
        "Convert all audio to {fmt}",
        "Tüm sesleri {fmt} olarak dönüştür",
    ),
    (
        "menu.folder_preset",
        "{name} (all images)",
        "{name} (tüm resimler)",
    ),
    (
        "notify.folder_empty",
        "No files to convert in this folder",
        "Bu klasörde dönüştürülecek dosya yok",
    ),
    (
        "notify.folder_cap",
        "Only the first {max} files of this folder are converted",
        "Bu klasörde yalnızca ilk {max} dosya dönüştürülüyor",
    ),
    (
        "notify.folder_bad",
        "That folder cannot be used",
        "Bu klasör kullanılamıyor",
    ),
    // dropped files
    (
        "drop.unsupported",
        "None of these files can be converted",
        "Bu dosyaların hiçbiri dönüştürülemez",
    ),
    (
        "drop.bad_choice",
        "That choice does not fit the selected files",
        "Bu seçim seçilen dosyalara uymuyor",
    ),
    ("drop.custom_name", "Custom settings", "Özel ayar"),
    ("menu.custom", "Custom settings...", "Özel ayar..."),
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
    (
        "cmd.export",
        "Could not export settings: {err}",
        "Ayarlar dışa aktarılamadı: {err}",
    ),
    (
        "cmd.import",
        "Could not import settings: {err}",
        "Ayarlar içe aktarılamadı: {err}",
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
        "The file is corrupt or unreadable. Try opening it in another program first.",
        "Dosya bozuk veya okunamıyor. Önce başka bir programda açmayı deneyin.",
    ),
    (
        "ce.unsupported",
        "This file cannot become that format. Pick another format.",
        "Bu dosya o biçime dönüştürülemiyor. Başka bir biçim seçin.",
    ),
    (
        "ce.trim_outside",
        "The cut starts after the end of the clip.",
        "Kesme başlangıcı klibin bitişinden sonra.",
    ),
    ("ce.too_large", "File too large", "Dosya çok büyük"),
    (
        "ce.target_size",
        "The size limit cannot be reached without making the result unusable. Try a larger limit.",
        "Boyut sınırına, sonuç kullanılamaz hale gelmeden ulaşılamıyor. Daha büyük bir sınır deneyin.",
    ),
    (
        "ce.protected",
        "The file is password protected",
        "Dosya parola korumalı",
    ),
    (
        "ce.needs_ffmpeg",
        "This conversion needs ffmpeg. Install it under Settings > Advanced.",
        "Bu dönüşüm için ffmpeg gerekiyor. Ayarlar > Gelişmiş bölümünden kurun.",
    ),
    (
        "ce.needs_office",
        "Office documents need document support. Download it under Settings > Advanced.",
        "Ofis belgeleri için belge desteği gerekiyor. Ayarlar > Gelişmiş bölümünden indirin.",
    ),
    ("ce.cancelled", "Cancelled", "İptal edildi"),
    (
        "ce.encode",
        "Conversion failed: {detail}. Try another target format.",
        "Dönüştürme başarısız: {detail}. Başka bir hedef biçim deneyin.",
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
        "The file has no audio or video track this conversion needs (for example no sound for MP3)",
        "Dosyada bu dönüşümün gerektiği ses ya da görüntü yok (ör. MP3 için ses)",
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
    // document support (LibreOffice) download errors
    (
        "lo.unpack",
        "Could not unpack document support: {detail}",
        "Belge desteği açılamadı: {detail}",
    ),
    (
        "lo.missing",
        "The document support package is incomplete",
        "Belge desteği paketi eksik",
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
