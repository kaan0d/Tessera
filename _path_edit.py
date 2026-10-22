def edit(p, pairs):
    s = open(p, encoding='utf8').read()
    for old, new in pairs:
        assert old in s, (p, old[:80])
        s = s.replace(old, new, 1)
    open(p, 'w', encoding='utf8').write(s)


part = open('_path_part.rs', encoding='utf8').read()
edit('src-tauri/src/install.rs', [
    ("/// What an install produced:", part + "/// What an install produced:"),
    ("    #[serde(default = \"yes\")]\n    pub ffmpeg: bool,\n}", "    #[serde(default = \"yes\")]\n    pub ffmpeg: bool,\n    /// Add the install folder to the user's PATH so `tessera-cli` works in a terminal.\n    #[serde(default)]\n    pub path: bool,\n}"),
    ("            write_uninstall_entry(&dir, &dst).map_err(fail(\"Cannot register the uninstaller\"))",
     "            if req.path {\n                if let Err(e) = add_to_path(&dir) {\n                    crate::log::write(\"warn\", &format!(\"PATH not updated: {e}\"));\n                }\n            }\n            write_uninstall_entry(&dir, &dst).map_err(fail(\"Cannot register the uninstaller\"))"),
    ("            if !done(RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(UNINSTALL_KEY)) {\n                left.push(\"entry\");\n            }",
     "            if !done(RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(UNINSTALL_KEY)) {\n                left.push(\"entry\");\n            }\n            if !done(remove_from_path(&dir)) {\n                left.push(\"path\");\n            }"),
])
edit('src-tauri/src/cli.rs', [
    ("    Install { dir: String, portable: bool },", "    Install {\n        dir: String,\n        portable: bool,\n        path: bool,\n    },"),
    ("                portable: args.iter().any(|a| a == \"--portable\"),\n            },", "                portable: args.iter().any(|a| a == \"--portable\"),\n                path: args.iter().any(|a| a == \"--add-to-path\"),\n            },"),
])
edit('src-tauri/src/lib.rs', [
    ("        Cli::Install { dir, portable } => {", "        Cli::Install { dir, portable, path } => {"),
    ("                ffmpeg: true,\n            };", "                ffmpeg: true,\n                path: *path,\n            };"),
])
# installer window and screen
edit('src/install.css', [("width:800px;height:520px;", "width:800px;height:580px;")])
edit('src/install.html', [
    ("""            <small data-t="ffdl_hint">Needed for video, MP3, OGG and AAC. You can also do it later.</small>
          </span>
        </label>""", """            <small data-t="ffdl_hint">Needed for video, MP3, OGG and AAC. You can also do it later.</small>
          </span>
        </label>
        <label class="check">
          <input type="checkbox" id="pathadd">
          <span class="box" aria-hidden="true"><svg viewBox="0 0 16 16"><path d="M3.5 8.5l3 3 6-7"/></svg></span>
          <span class="check-t"><span data-t="path">Add to PATH for terminals</span>
            <small data-t="path_hint">Lets you type tessera-cli in Command Prompt or PowerShell. Most people do not need it.</small>
          </span>
        </label>"""),
    ('        <p class="fine warn" id="ffNote"></p>', '        <p class="fine warn" id="ffNote"></p>\n        <p class="fine" id="pathNote"></p>'),
])
edit('src/install.js', [
    ('      ffdl: "Download video support now (about 170 MB)",', '      path: "Add to PATH for terminals",\n      path_hint: "Lets you type tessera-cli in Command Prompt or PowerShell. Most people do not need it.",\n      path_note: "PATH updated. Open a new terminal window to use tessera-cli.",\n      left_path: "the PATH entry",\n      ffdl: "Download video support now (about 170 MB)",'),
    ('      ffdl: "Video desteğini şimdi indir (yaklaşık 170 MB)",', '      path: "Terminaller için PATH\'e ekle",\n      path_hint: "Komut İstemi ve PowerShell\'de tessera-cli yazabilirsin. Çoğu kişiye gerekmez.",\n      path_note: "PATH güncellendi. tessera-cli için yeni bir terminal penceresi aç.",\n      left_path: "PATH kaydı",\n      ffdl: "Video desteğini şimdi indir (yaklaşık 170 MB)",'),
    ('ffmpeg: document.getElementById("ffdl").checked } }', 'ffmpeg: document.getElementById("ffdl").checked, path: document.getElementById("pathadd").checked } }'),
    ('      .then((real) => { dirEl.value = real;', '      .then((real) => { document.getElementById("pathNote").textContent = document.getElementById("pathadd").checked ? t("path_note") : ""; dirEl.value = real;'),
])
import json
p = 'src-tauri/tauri.conf.json'
c = json.load(open(p, encoding='utf8'))
for w in c['app']['windows']:
    if w['label'] == 'setup':
        w['height'] = 580
json.dump(c, open(p, 'w', encoding='utf8'), indent=2)
open(p, 'a').write('\n')
print('ok')
