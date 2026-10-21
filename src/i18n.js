'use strict';
/* UI strings in English and Turkish. Add a string: one key in both `en` and `tr`
   (development/check-i18n.js fails otherwise), then use it with
   t('key', {vars}), data-i18n="key", data-i18n-html="key" or
   data-i18n-attr="aria-label:key;title:key2" in the HTML.
   Plurals: define `key.one` and `key.other`, call t('key', {n}).
   {placeholders} must match between the languages. The app name is not in here. */
const I18N = {
  en: {
    'app.settings': 'Settings',
    'saved': 'Saved',

    'tabs.label': 'Settings sections',
    'tab.general': 'General',
    'tab.formats': 'Formats',
    'tab.output': 'Output',
    'tab.quality': 'Quality',
    'tab.panel': 'Panel',
    'tab.menu': 'Menu',
    'tab.advanced': 'Advanced',

    'ffn.title': 'Getting video and audio support ready',
    'ffn.body': 'ffmpeg, a free open-source tool, is downloaded once (about 170 MB) for video, MP3, OGG and AAC. You can keep converting images, WAV and FLAC while you wait.',
    'ffn.ok': 'Got it',

    'lang.label': 'Language',
    'lang.desc': 'Language of the app and its right-click menu.',
    'autostart.label': 'Start with Windows',
    'autostart.desc': 'Waits in the system tray after you sign in.',
    'general.aside': 'Right-click a file and choose a format from the <b>Convert</b> menu. You can change which formats appear under <button type="button" class="link" data-goto="t-bicim">Formats</button>.',

    'kind.image': 'Image',
    'kind.audio': 'Audio',
    'kind.video': 'Video',
    'fmt.targets': 'Formats you can convert to',
    'fmt.sources': 'File types that get the menu',
    'all.btn': 'All',
    'none.btn': 'None',
    'all.aria': '{kind}: all',
    'none.aria': '{kind}: none',
    'chip.locked': 'Available once ffmpeg is downloaded',
    'chip.ff': 'Converted with ffmpeg',
    'hint.none_targets': 'No format is selected. The right-click menu will not offer any conversion.',
    'hint.ff_wait': ' marked formats need ffmpeg. They unlock when the download finishes. ',
    'hint.ff_ready': ' marked formats are converted with ffmpeg.',
    'hint.see_status': 'See status',
    'hint.sources.one': 'The menu appears on {n} file type.',
    'hint.sources.other': 'The menu appears on {n} file types.',
    'hint.none_sources': 'No type is selected. The right-click menu will not appear on any file.',
    'flash.ff_needed': '{fmt} needs ffmpeg. It unlocks when the download finishes.',
    'flash.last': 'At least one format must stay on.',

    'out.name': 'File name',
    'out.name_desc': 'The extension is added automatically. Clicking a tag inserts it at the cursor.',
    'out.tags': 'Tags',
    'tok.name': 'name',
    'tok.format': 'format',
    'tok.date': 'date',
    'tpl.example_src': 'photo.png',
    'tpl.sample': 'photo',
    'tpl.empty': 'The file name cannot be empty.',
    'tpl.chars': 'These characters are not allowed: \\ / : * ? " < > |',
    'tpl.unknown': 'Unknown tag: {tag}. Available: {name}, {format}, {date}.',
    'tpl.braces': 'The curly braces do not match.',
    'tpl.trailing': 'The file name cannot end with a dot or a space.',
    'out.where': 'Save location',
    'out.same': 'Next to the source file',
    'out.fixed': 'In a fixed folder',
    'out.pick': 'Choose folder',
    'out.conflict': 'If a file with the same name exists',
    'out.opt_rename': 'Add a number: file (1)',
    'out.opt_overwrite': 'Overwrite',
    'out.opt_replace': 'Replace the original',
    'out.warn': 'When the conversion finishes, the original file is moved to the Recycle Bin and the new file takes its place.',

    'q.small': 'Smaller file',
    'q.high': 'Higher quality',
    'q.audio_foot': 'Values are average bit rates and vary slightly with the audio content.',
    'q.videogif': 'Video and GIF',
    'q.video_rate': 'Video bit rate',
    'q.gif_fps': 'GIF frame rate',
    'q.gif_width': 'GIF width',
    'q.gif_desc': 'A higher frame rate and width make the GIF smoother, and the file grows quickly.',

    'pn.pos': 'Position on screen',
    'pn.preview': 'Preview',
    'pn.pos_desc': 'Click the corner where the panel should appear. Preview shows the real panel on your desktop.',
    'pn.top_left': 'Top left',
    'pn.top_right': 'Top right',
    'pn.bottom_left': 'Bottom left',
    'pn.bottom_right': 'Bottom right',
    'pn.stay': 'Stay on screen when done',
    'pn.stay_lo': 'Close right away',
    'pn.stay_hi': 'Stay longer',
    'pn.toast': 'Windows notification',
    'pn.toast_desc': 'A short summary is also sent to the notification center when conversions finish, so you can still see it after the panel closes.',
    'unit.sec': 's',

    'mn.icons': 'Format icons',
    'mn.icons_desc': 'Each format in the menu gets a small icon showing its type.',
    'mn.sort': 'Order',
    'mn.sort_desc': 'The order of formats in the menu.',
    'mn.default': 'Default',
    'mn.explorer': 'Explorer menu',
    'mn.explorer_desc': 'In Windows 11 it appears under "Show more options".',
    'mn.reinstall': 'Register again',
    'mn.remove': 'Remove',
    'mn.added': 'Menu registered.',
    'mn.removed': 'Menu removed.',

    'adv.videoaudio': 'Video and audio',
    'ff.initial': 'Checking status…',
    'ff.bar': 'ffmpeg download progress',
    'ff.retry': 'Try again',
    'ff.foot': 'Free and open source, downloaded automatically. Images, WAV and FLAC convert without it.',
    'ff.ready': 'Ready. Video and audio files can be converted.',
    'ff.downloading': 'Downloading in the background.',
    'ff.failed': 'Download failed: {err}',
    'ff.checking': 'Checking. The download starts shortly.',
    'ff.status_err': 'Could not get the ffmpeg status: {err}',

    'modal.title': 'Replace the original files?',
    'modal.text': 'After every conversion, the original file is moved to the Recycle Bin and the new file takes its place.',
    'modal.no': 'Cancel',
    'modal.yes': 'Yes, turn on',

    'pct': '{n}%',
    'err.backend': 'Could not reach the app backend.',
    'err.save': 'Could not save settings: {err}',
    'err.load': 'Could not load settings: {err}',
    'err.catalog': 'Could not load the format list: {err}',
    'err.preview': 'Could not open the preview: {err}',
    'err.no_dialog': 'The folder picker is not available.',
    'err.pick': 'Could not choose the folder: {err}',
    'err.action': 'Action failed: {err}',
    'err.dl_start': 'Could not start the download: {err}',

    'p.running': 'Converting',
    'p.some_failed': 'Some files could not be converted',
    'p.done': 'Done',
    'p.sub_one': '1 file',
    'p.sub_run': '{done}/{total} files done',
    'p.sub_fail': '{ok} succeeded, {err} failed',
    'p.sub_ok.one': '{n} file ready',
    'p.sub_ok.other': '{n} files ready',
    'p.list': 'Conversions',
    'p.cancel': 'Cancel {name}',
    'p.cancelling': 'Cancelling…',
    'p.fail_default': 'The conversion failed.',
    'p.open': 'Open folder',
    'p.close': 'Close',
    'p.open_err': 'Could not open the folder: {err}',
    'p.ann_ok': '{name} converted.',
    'p.ann_err': '{name} could not be converted.',
  },

  tr: {
    'app.settings': 'Ayarlar',
    'saved': 'Kaydedildi',

    'tabs.label': 'Ayar bölümleri',
    'tab.general': 'Genel',
    'tab.formats': 'Biçimler',
    'tab.output': 'Çıktı',
    'tab.quality': 'Kalite',
    'tab.panel': 'Panel',
    'tab.menu': 'Menü',
    'tab.advanced': 'Gelişmiş',

    'ffn.title': 'Video ve ses desteği hazırlanıyor',
    'ffn.body': 'Video, MP3, OGG ve AAC için ücretsiz ve açık kaynaklı ffmpeg bir kez indirilir (yaklaşık 170 MB). Beklerken resim, WAV ve FLAC dönüştürmeye devam edebilirsiniz.',
    'ffn.ok': 'Anladım',

    'lang.label': 'Dil',
    'lang.desc': 'Uygulamanın ve sağ tık menüsünün dili.',
    'autostart.label': 'Windows ile başlat',
    'autostart.desc': 'Oturum açıldığında simge tepsisinde hazır bekler.',
    'general.aside': 'Dosyalara sağ tıklayıp <b>Dönüştür</b> menüsünden bir biçim seçin. Hangi biçimlerin görüneceğini <button type="button" class="link" data-goto="t-bicim">Biçimler</button> bölümünden değiştirebilirsiniz.',

    'kind.image': 'Görsel',
    'kind.audio': 'Ses',
    'kind.video': 'Video',
    'fmt.targets': 'Dönüştürülebilecek biçimler',
    'fmt.sources': 'Menüde görünecek dosya türleri',
    'all.btn': 'Tümü',
    'none.btn': 'Hiçbiri',
    'all.aria': '{kind}: tümü',
    'none.aria': '{kind}: hiçbiri',
    'chip.locked': 'ffmpeg indirilince kullanılabilir',
    'chip.ff': 'ffmpeg ile dönüştürülür',
    'hint.none_targets': 'Hiçbir biçim seçili değil. Sağ tık menüsünde dönüştürme seçeneği görünmez.',
    'hint.ff_wait': ' işaretli biçimler ffmpeg ister. İndirme bitince kendiliğinden açılır. ',
    'hint.ff_ready': ' işaretli biçimler ffmpeg ile dönüştürülür.',
    'hint.see_status': 'Durumu gör',
    'hint.sources.one': 'Menü {n} dosya türünde görünür.',
    'hint.sources.other': 'Menü {n} dosya türünde görünür.',
    'hint.none_sources': 'Hiçbir tür seçili değil. Sağ tık menüsü hiçbir dosyada görünmez.',
    'flash.ff_needed': '{fmt} için ffmpeg gerekiyor. İndirme bitince açılır.',
    'flash.last': 'En az bir biçim açık kalmalı.',

    'out.name': 'Dosya adı',
    'out.name_desc': 'Uzantı kendiliğinden eklenir. Etikete tıklayınca imlecin olduğu yere girer.',
    'out.tags': 'Etiketler',
    'tok.name': 'ad',
    'tok.format': 'biçim',
    'tok.date': 'tarih',
    'tpl.example_src': 'foto.png',
    'tpl.sample': 'foto',
    'tpl.empty': 'Dosya adı boş olamaz.',
    'tpl.chars': 'Şu karakterler kullanılamaz: \\ / : * ? " < > |',
    'tpl.unknown': 'Bilinmeyen etiket: {tag}. Kullanılabilenler: {name}, {format}, {date}.',
    'tpl.braces': 'Süslü parantezler eşleşmiyor.',
    'tpl.trailing': 'Dosya adı nokta veya boşlukla bitemez.',
    'out.where': 'Kayıt konumu',
    'out.same': 'Kaynak dosyanın yanına',
    'out.fixed': 'Sabit bir klasöre',
    'out.pick': 'Klasör seç',
    'out.conflict': 'Aynı adlı dosya varsa',
    'out.opt_rename': 'Numara ekle: dosya (1)',
    'out.opt_overwrite': 'Üzerine yaz',
    'out.opt_replace': 'Orijinalin yerine koy',
    'out.warn': "Dönüştürme bitince orijinal dosya Geri Dönüşüm Kutusu'na taşınır, yerine yenisi konur.",

    'q.small': 'Küçük dosya',
    'q.high': 'Yüksek kalite',
    'q.audio_foot': 'Değerler ortalama bit hızıdır. Sesin içeriğine göre biraz değişir.',
    'q.videogif': 'Video ve GIF',
    'q.video_rate': 'Video bit hızı',
    'q.gif_fps': 'GIF kare hızı',
    'q.gif_width': 'GIF genişliği',
    'q.gif_desc': 'Kare hızı ve genişlik arttıkça GIF akıcılaşır, dosya da hızla büyür.',

    'pn.pos': 'Ekrandaki yeri',
    'pn.preview': 'Önizle',
    'pn.pos_desc': 'Panelin açılacağı köşeye tıklayın. Önizle, gerçek paneli masaüstünde gösterir.',
    'pn.top_left': 'Sol üst',
    'pn.top_right': 'Sağ üst',
    'pn.bottom_left': 'Sol alt',
    'pn.bottom_right': 'Sağ alt',
    'pn.stay': 'Bitince ekranda kalma',
    'pn.stay_lo': 'Hemen kapansın',
    'pn.stay_hi': 'Uzun kalsın',
    'pn.toast': 'Windows bildirimi',
    'pn.toast_desc': 'Dönüştürmeler bitince bildirim merkezine kısa bir özet de gönderilir. Panel kapandıktan sonra da oradan görebilirsiniz.',
    'unit.sec': 'sn',

    'mn.icons': 'Biçim simgeleri',
    'mn.icons_desc': 'Menüde her biçimin yanında türünü gösteren küçük bir simge olur.',
    'mn.sort': 'Sıralama',
    'mn.sort_desc': 'Menüdeki biçimlerin sırası.',
    'mn.default': 'Varsayılan',
    'mn.explorer': 'Explorer menüsü',
    'mn.explorer_desc': 'Windows 11\'de "Daha fazla seçenek göster" altında görünür.',
    'mn.reinstall': 'Yeniden kaydet',
    'mn.remove': 'Kaldır',
    'mn.added': 'Menü kaydedildi.',
    'mn.removed': 'Menü kaldırıldı.',

    'adv.videoaudio': 'Video ve ses',
    'ff.initial': 'Durum denetleniyor…',
    'ff.bar': 'ffmpeg indirme ilerlemesi',
    'ff.retry': 'Yeniden dene',
    'ff.foot': 'Ücretsiz ve açık kaynaklı, kendiliğinden indirilir. Resim, WAV ve FLAC onsuz da dönüştürülür.',
    'ff.ready': 'Hazır. Video ve ses dosyaları dönüştürülebilir.',
    'ff.downloading': 'İndiriliyor, arka planda sürüyor.',
    'ff.failed': 'İndirilemedi: {err}',
    'ff.checking': 'Denetleniyor, indirme birazdan başlar.',
    'ff.status_err': 'ffmpeg durumu alınamadı: {err}',

    'modal.title': 'Orijinal dosya silinsin mi?',
    'modal.text': "Her dönüştürmeden sonra orijinal dosya Geri Dönüşüm Kutusu'na taşınır ve yerine yeni dosya konur.",
    'modal.no': 'Vazgeç',
    'modal.yes': 'Evet, aç',

    'pct': '%{n}',
    'err.backend': 'Uygulama arka ucuna bağlanamadı.',
    'err.save': 'Ayarlar kaydedilemedi: {err}',
    'err.load': 'Ayarlar yüklenemedi: {err}',
    'err.catalog': 'Biçim listesi alınamadı: {err}',
    'err.preview': 'Önizleme açılamadı: {err}',
    'err.no_dialog': 'Klasör seçme penceresi kullanılamıyor.',
    'err.pick': 'Klasör seçilemedi: {err}',
    'err.action': 'İşlem başarısız: {err}',
    'err.dl_start': 'İndirme başlatılamadı: {err}',

    'p.running': 'Dönüştürülüyor',
    'p.some_failed': 'Bazı dosyalar dönüştürülemedi',
    'p.done': 'Tamamlandı',
    'p.sub_one': '1 dosya',
    'p.sub_run': '{done}/{total} dosya bitti',
    'p.sub_fail': '{ok} başarılı, {err} başarısız',
    'p.sub_ok.one': '{n} dosya hazır',
    'p.sub_ok.other': '{n} dosya hazır',
    'p.list': 'Dönüştürmeler',
    'p.cancel': 'İptal et: {name}',
    'p.cancelling': 'İptal ediliyor…',
    'p.fail_default': 'Dönüştürme başarısız oldu.',
    'p.open': 'Klasörü aç',
    'p.close': 'Kapat',
    'p.open_err': 'Klasör açılamadı: {err}',
    'p.ann_ok': '{name} dönüştürüldü.',
    'p.ann_err': '{name} dönüştürülemedi.',
  },
};

let LANG = 'en';

/** Text for `key` in the current language, `{vars}` filled in. */
function t(key, vars) {
  const d = I18N[LANG];
  let k = key;
  if (vars && typeof vars.n === 'number') {
    const p = key + (vars.n === 1 ? '.one' : '.other');
    if (p in d) k = p;
  }
  let s = k in d ? d[k] : k in I18N.en ? I18N.en[k] : key;
  if (vars) for (const v in vars) s = s.replaceAll('{' + v + '}', () => vars[v]);
  return s;
}

/** Switches the language and rewrites every marked element. */
function setLang(code) {
  LANG = code === 'tr' ? 'tr' : 'en';
  document.documentElement.lang = LANG;
  applyI18n();
}

function applyI18n(root = document) {
  for (const e of root.querySelectorAll('[data-i18n]')) e.textContent = t(e.dataset.i18n);
  for (const e of root.querySelectorAll('[data-i18n-html]')) e.innerHTML = t(e.dataset.i18nHtml); // dictionary text only
  for (const e of root.querySelectorAll('[data-i18n-attr]')) {
    for (const pair of e.dataset.i18nAttr.split(';')) {
      const [attr, key] = pair.split(':');
      e.setAttribute(attr.trim(), t(key.trim()));
    }
  }
}

if (typeof module !== 'undefined') module.exports = { I18N };
