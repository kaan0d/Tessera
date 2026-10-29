//! Backend strings, fr. Same keys and {placeholders} as `MSGS` (a test checks).

pub const MSGS: &[(&str, &str)] = &[
    (
        "ff.busy",
        "ffmpeg est en cours de téléchargement ; supprimez-le une fois le téléchargement terminé",
    ),
    ("setup.title", "Installation de Tessera"),
    ("setup.uninstall_title", "Désinstaller Tessera"),
    ("menu.convert_to", "Convertir en {fmt}"),
    ("pdf.split", "Fractionner en pages"),
    ("pdf.extract", "Extraire des pages"),
    ("pdf.delete", "Supprimer des pages"),
    ("pdf.rotate_right", "Faire pivoter à droite"),
    ("pdf.rotate_left", "Faire pivoter à gauche"),
    ("pdf.rotate_180", "Retourner"),
    ("pdf.compress", "Compresser le PDF"),
    ("menu.pdf_more", "Autres outils PDF..."),
    ("menu.merge_pdf", "Fusionner en PDF"),
    ("menu.merge_audio", "Fusionner l'audio en un fichier"),
    ("preset.web", "Alléger pour le web"),
    ("preset.whatsapp", "Pour WhatsApp"),
    ("preset.email", "E-mail : moins de 10 Mo"),
    ("preset.discord", "Discord : moins de 10 Mo"),
    ("preset.strip", "Supprimer position et infos d'appareil"),
    ("preset.pdf", "Créer un PDF"),
    ("preset.webp", "Créer un WebP"),
    ("preset.ico", "Créer une icône (ICO)"),
    ("preset.instagram", "Pour Instagram (1080 px)"),
    ("preset.podcast", "Podcast : MP3 mono, 96 kbps"),
    (
        "preset.bad_name",
        "Le nom du préréglage doit comporter de 1 à 40 caractères",
    ),
    (
        "preset.bad_id",
        "L'identifiant du préréglage ne peut contenir que a-z, 0-9, - et _ (32 caractères au maximum)",
    ),
    (
        "preset.builtin",
        "Les préréglages intégrés ne peuvent pas être modifiés",
    ),
    (
        "preset.bad_trim",
        "La fin de la coupe doit se situer après son début",
    ),
    (
        "preset.bad_kinds",
        "Choisissez au moins un type de fichier pour le préréglage",
    ),
    (
        "preset.bad_target",
        "Le préréglage a un format cible inconnu",
    ),
    (
        "preset.same_needs_option",
        "Pour conserver le format, il faut un redimensionnement, une limite de taille ou la suppression de la position et des infos d'appareil photo",
    ),
    (
        "preset.too_many",
        "Vous pouvez avoir au maximum {max} préréglages personnalisés",
    ),
    ("preset.not_found", "Préréglage introuvable"),
    (
        "notify.preset_skip",
        "{n} fichiers ignorés : non pris en charge par {name}",
    ),
    ("job.merge_name", "Fusion de {n} fichiers"),
    (
        "undo.not_found",
        "Cette entrée de l'historique n'existe plus",
    ),
    ("undo.nothing", "Il n'y a rien à annuler pour cette entrée"),
    ("undo.no_output", "Le fichier converti n'existe déjà plus"),
    (
        "undo.changed",
        "Le fichier converti a été modifié après sa création ; il n'a donc pas été touché",
    ),
    (
        "undo.occupied",
        "Un fichier existe déjà à l'emplacement d'origine : {path}",
    ),
    (
        "undo.restore_failed",
        "Impossible de restaurer l'original ({detail}). Il se trouve toujours dans la Corbeille : {path}",
    ),
    (
        "undo.delete_failed",
        "Impossible de déplacer le fichier converti dans la Corbeille : {detail}",
    ),
    (
        "update.available",
        "La version {latest} est disponible (vous avez la {current})",
    ),
    ("update.up_to_date", "Vous êtes à jour (version {current})"),
    (
        "update.none_yet",
        "Aucune version n'a encore été publiée, vous êtes donc à jour",
    ),
    (
        "update.failed",
        "Impossible de rechercher des mises à jour : {err}",
    ),
    (
        "update.not_installed",
        "La mise à jour ne fonctionne que depuis l'application installée. Téléchargez la nouvelle version depuis la page de publication.",
    ),
    (
        "update.no_asset",
        "Cette version ne comporte aucun programme d'installation à télécharger",
    ),
    (
        "update.bad_response",
        "Le serveur de mise à jour a renvoyé une réponse inattendue",
    ),
    ("update.bad_url", "Ce lien ne peut pas être ouvert"),
    ("update.open_failed", "Impossible d'ouvrir le navigateur"),
    ("update.title", "Mise à jour disponible"),
    (
        "watch.too_many",
        "Vous pouvez avoir au maximum {max} règles de surveillance",
    ),
    (
        "watch.bad_id",
        "Une règle de surveillance a un identifiant non valide ou en double",
    ),
    (
        "watch.bad_folder",
        "Choisissez un dossier sur un lecteur local : {folder}",
    ),
    (
        "watch.forbidden_folder",
        "Ce dossier ne peut pas être utilisé pour la surveillance ou la sortie : {folder}",
    ),
    (
        "watch.bad_ext",
        "Type de fichier inconnu dans une règle de surveillance : {ext}",
    ),
    (
        "watch.bad_output",
        "Une règle de surveillance a un emplacement de sortie non valide",
    ),
    ("watch.missing_folder", "Le dossier n'existe pas : {folder}"),
    (
        "watch.start_failed",
        "Impossible de surveiller le dossier : {err}",
    ),
    ("watch.label", "Surveillance : {folder}"),
    (
        "cli.no_ffmpeg",
        "Cette conversion nécessite ffmpeg, qui n'est pas installé. Lancez l'application une fois pour qu'elle puisse l'installer.",
    ),
    (
        "cli.no_office",
        "Les documents Office nécessitent la prise en charge des documents, qui n'est pas téléchargée. Obtenez-la dans l'application sous Paramètres > Avancé, ou exécutez `tessera --install-office`.",
    ),
    ("tray.open", "Paramètres"),
    ("tray.quit", "Quitter"),
    ("notify.dl_title", "Téléchargement de ffmpeg"),
    (
        "notify.dl_body",
        "La conversion vidéo et MP3/OGG nécessite ffmpeg, un outil libre et gratuit. Il sera téléchargé une seule fois (environ 170 Mo).",
    ),
    ("notify.ready_title", "ffmpeg est prêt"),
    (
        "notify.ready_body",
        "La conversion vidéo et audio est prête à l'emploi.",
    ),
    ("notify.done_title", "Converti"),
    ("notify.failed_title", "Échec de la conversion"),
    (
        "notify.target_off",
        "{fmt} est désactivé dans les paramètres",
    ),
    (
        "menu.folder_images_to",
        "Convertir toutes les images en {fmt}",
    ),
    ("menu.folder_audio_to", "Convertir tout l'audio en {fmt}"),
    (
        "menu.folder_docs_to",
        "Convertir tous les documents en {fmt}",
    ),
    ("menu.folder_preset", "{name} (toutes les images)"),
    (
        "notify.folder_empty",
        "Aucun fichier à convertir dans ce dossier",
    ),
    (
        "notify.folder_cap",
        "Seuls les {max} premiers fichiers de ce dossier sont convertis",
    ),
    ("notify.folder_bad", "Ce dossier ne peut pas être utilisé"),
    (
        "drop.unsupported",
        "Aucun de ces fichiers ne peut être converti",
    ),
    (
        "drop.bad_choice",
        "Ce choix ne convient pas aux fichiers sélectionnés",
    ),
    ("drop.custom_name", "Paramètres personnalisés"),
    ("menu.custom", "Paramètres personnalisés..."),
    (
        "cmd.autostart",
        "Impossible de modifier le lancement au démarrage : {err}",
    ),
    (
        "cmd.menu_save",
        "Impossible d'enregistrer le menu contextuel : {err}",
    ),
    (
        "cmd.menu_remove",
        "Impossible de supprimer le menu contextuel : {err}",
    ),
    (
        "cmd.save",
        "Impossible d'enregistrer les paramètres : {err}",
    ),
    ("cmd.export", "Impossible d'exporter les paramètres : {err}"),
    ("cmd.import", "Impossible d'importer les paramètres : {err}"),
    ("cmd.bad_corner", "Coin non valide"),
    ("cmd.bad_seconds", "Durée non valide"),
    ("cmd.bad_path", "Chemin non valide"),
    ("cmd.no_path", "Chemin introuvable"),
    ("job.done", "Terminé"),
    ("job.cancelled", "Annulé"),
    (
        "job.needs_ffmpeg",
        "Cette conversion nécessite ffmpeg, qui n'est pas encore prêt",
    ),
    ("panel.demo_name", "exemple-photo.png"),
    (
        "tpl.length",
        "Le nom de fichier doit comporter de 1 à {max} caractères",
    ),
    (
        "tpl.unknown_token",
        "Le nom de fichier contient une balise inconnue ; seules {name}, {format} et {date} sont autorisées",
    ),
    (
        "tpl.bad_char",
        "Le nom de fichier contient un caractère non valide : {c}",
    ),
    (
        "tpl.trailing",
        "Le nom de fichier ne peut pas se terminer par un point ou un espace",
    ),
    (
        "tpl.reserved",
        "Le nom de fichier serait un nom réservé de Windows",
    ),
    ("ce.invalid_path", "Chemin de fichier non valide : {detail}"),
    ("ce.not_found", "Fichier introuvable"),
    ("ce.access_denied", "Accès refusé"),
    ("ce.disk_full", "Disque plein"),
    (
        "ce.corrupt",
        "Le fichier est endommagé ou illisible. Essayez d'abord de l'ouvrir dans un autre programme.",
    ),
    (
        "ce.unsupported",
        "Ce fichier ne peut pas être converti dans ce format. Choisissez-en un autre.",
    ),
    ("ce.trim_outside", "La coupe commence après la fin du clip."),
    ("ce.too_large", "Fichier trop volumineux"),
    (
        "ce.target_size",
        "La limite de taille ne peut pas être atteinte sans rendre le résultat inutilisable. Essayez une limite plus élevée.",
    ),
    ("ce.protected", "Le fichier est protégé par un mot de passe"),
    (
        "ce.needs_ffmpeg",
        "Cette conversion nécessite ffmpeg. Installez-le sous Paramètres > Avancé.",
    ),
    (
        "ce.bad_pages",
        "Vérifiez les numéros de page : ce PDF compte {n} pages. Écrivez-les sous la forme 1-3, 5.",
    ),
    (
        "ce.needs_office",
        "Les documents Office nécessitent la prise en charge des documents. Téléchargez-la sous Paramètres > Avancé.",
    ),
    ("ce.cancelled", "Annulé"),
    (
        "ce.encode",
        "Échec de la conversion : {detail}. Essayez un autre format cible.",
    ),
    ("ce.io", "Erreur de fichier : {detail}"),
    ("ce.not_file", "pas un fichier"),
    ("ce.not_dir", "pas un dossier"),
    ("ce.network_path", "chemin réseau ou de périphérique"),
    ("ce.bad_name", "nom de fichier non valide"),
    ("ce.no_free_name", "aucun nom de fichier disponible"),
    (
        "ce.no_stream",
        "Le fichier n'a pas la piste audio ou vidéo nécessaire à cette conversion (par exemple pas de son pour un MP3)",
    ),
    ("ce.empty_output", "Sortie vide"),
    ("ce.unknown", "erreur inconnue"),
    ("ce.ffmpeg_start", "Impossible de lancer ffmpeg : {detail}"),
    ("ce.ffmpeg_wait", "Impossible d'attendre ffmpeg : {detail}"),
    (
        "ce.recycle",
        "Impossible de déplacer le fichier d'origine dans la Corbeille : {detail}",
    ),
    ("ff.read", "Impossible de lire le fichier : {detail}"),
    (
        "ff.hash",
        "Le fichier téléchargé n'a pas pu être vérifié (SHA-256 non concordant)",
    ),
    (
        "ff.start",
        "Impossible de lancer le téléchargement : {detail}",
    ),
    (
        "ff.too_big",
        "Le fichier à télécharger est plus volumineux que prévu",
    ),
    (
        "ff.tmp",
        "Impossible de créer un fichier temporaire : {detail}",
    ),
    ("ff.cut", "Le téléchargement a été interrompu : {detail}"),
    ("ff.write", "Impossible d'écrire sur le disque : {detail}"),
    ("ff.archive", "Impossible d'ouvrir l'archive : {detail}"),
    (
        "ff.unsafe",
        "L'archive contient un chemin de fichier dangereux",
    ),
    ("ff.no_exe", "ffmpeg.exe est introuvable dans l'archive"),
    ("ff.exe_big", "ffmpeg.exe est plus volumineux que prévu"),
    ("ff.mkdir", "Impossible de créer le dossier : {detail}"),
    ("ff.run", "Impossible d'exécuter ffmpeg"),
    (
        "lo.unpack",
        "Impossible de décompresser la prise en charge des documents : {detail}",
    ),
    (
        "lo.missing",
        "Le paquet de prise en charge des documents est incomplet",
    ),
    (
        "ce.bad_pages_one",
        "Vérifiez les numéros de page : ce PDF n'a qu'une page.",
    ),
    (
        "ce.no_pages_left",
        "Cela supprimerait toutes les pages. Gardez-en au moins une.",
    ),
    ("ce.empty_file", "Le fichier est vide"),
    (
        "ce.office_timeout",
        "LibreOffice n'a pas terminé en 5 minutes. Le document est probablement trop volumineux.",
    ),
    (
        "ce.no_recycle",
        "L'original a été conservé : ce lecteur n'a pas de Corbeille",
    ),
    ("ce.exists", "Un fichier portant ce nom existe déjà"),
    (
        "preset.kind_target",
        "Le format choisi ne convient pas à tous les types de fichiers de ce préréglage.",
    ),
    (
        "preset.dup_name",
        "Vous avez déjà un préréglage portant ce nom.",
    ),
    (
        "notify.preset_skip_one",
        "1 fichier ignoré : non pris en charge par {name}",
    ),
];
