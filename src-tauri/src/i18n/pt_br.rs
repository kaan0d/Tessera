//! Backend strings, pt-BR. Same keys and {placeholders} as `MSGS` (a test checks).

pub const MSGS: &[(&str, &str)] = &[
    (
        "ff.busy",
        "O ffmpeg está sendo baixado; remova-o quando o download terminar",
    ),
    ("setup.title", "Instalação do Tessera"),
    ("setup.uninstall_title", "Desinstalar o Tessera"),
    ("menu.convert_to", "Converter em {fmt}"),
    ("pdf.split", "Dividir em páginas"),
    ("pdf.extract", "Extrair páginas"),
    ("pdf.delete", "Excluir páginas"),
    ("pdf.rotate_right", "Girar para a direita"),
    ("pdf.rotate_left", "Girar para a esquerda"),
    ("pdf.rotate_180", "Virar de cabeça para baixo"),
    ("pdf.compress", "Compactar PDF"),
    ("menu.pdf_more", "Mais ferramentas de PDF..."),
    ("menu.merge_pdf", "Mesclar em PDF"),
    ("menu.merge_audio", "Juntar áudio em um arquivo"),
    ("preset.web", "Reduzir para a web"),
    ("preset.whatsapp", "Para WhatsApp"),
    ("preset.email", "Email: menos de 10 MB"),
    ("preset.discord", "Discord: menos de 10 MB"),
    ("preset.strip", "Remover local e dados da câmera"),
    ("preset.pdf", "Criar um PDF"),
    ("preset.webp", "Criar um WebP"),
    ("preset.ico", "Criar um ícone (ICO)"),
    ("preset.instagram", "Para Instagram (1080 px)"),
    ("preset.podcast", "Podcast: MP3 mono, 96 kbps"),
    (
        "preset.bad_name",
        "O nome da predefinição deve ter de 1 a 40 caracteres",
    ),
    (
        "preset.bad_id",
        "O id da predefinição só pode usar a-z, 0-9, - e _ (até 32 caracteres)",
    ),
    (
        "preset.builtin",
        "As predefinições internas não podem ser alteradas",
    ),
    ("preset.bad_trim", "O corte deve terminar depois de começar"),
    (
        "preset.bad_kinds",
        "Escolha pelo menos um tipo de arquivo para a predefinição",
    ),
    (
        "preset.bad_target",
        "A predefinição tem um formato de destino desconhecido",
    ),
    (
        "preset.same_needs_option",
        "Para manter o formato, é preciso redimensionar, limitar o tamanho ou remover o local e os dados da câmera",
    ),
    (
        "preset.too_many",
        "Você pode ter no máximo {max} predefinições personalizadas",
    ),
    ("preset.not_found", "Predefinição não encontrada"),
    (
        "notify.preset_skip",
        "{n} arquivos ignorados: sem suporte em {name}",
    ),
    ("job.merge_name", "Mesclando {n} arquivos"),
    (
        "undo.not_found",
        "Essa entrada do histórico não existe mais",
    ),
    ("undo.nothing", "Não há nada para desfazer nesta entrada"),
    ("undo.no_output", "O arquivo convertido já não existe"),
    (
        "undo.changed",
        "O arquivo convertido foi alterado depois de criado, então não foi mexido",
    ),
    (
        "undo.occupied",
        "Já existe um arquivo no local original: {path}",
    ),
    (
        "undo.restore_failed",
        "Não foi possível restaurar o original ({detail}). Ele ainda está na Lixeira: {path}",
    ),
    (
        "undo.delete_failed",
        "Não foi possível mover o arquivo convertido para a Lixeira: {detail}",
    ),
    (
        "update.available",
        "A versão {latest} está disponível (você tem a {current})",
    ),
    (
        "update.up_to_date",
        "Você está atualizado (versão {current})",
    ),
    (
        "update.none_yet",
        "Nenhuma versão foi publicada ainda, então você está atualizado",
    ),
    (
        "update.failed",
        "Não foi possível verificar se há atualizações: {err}",
    ),
    (
        "update.not_installed",
        "A atualização só funciona no aplicativo instalado. Baixe a nova versão na página de lançamentos.",
    ),
    (
        "update.no_asset",
        "Esta versão não tem um instalador para baixar",
    ),
    (
        "update.bad_response",
        "O servidor de atualização enviou uma resposta inesperada",
    ),
    ("update.bad_url", "Este link não pode ser aberto"),
    ("update.open_failed", "Não foi possível abrir o navegador"),
    ("update.title", "Atualização disponível"),
    (
        "watch.too_many",
        "Você pode ter no máximo {max} regras de monitoramento",
    ),
    (
        "watch.bad_id",
        "Uma regra de monitoramento tem um id inválido ou duplicado",
    ),
    (
        "watch.bad_folder",
        "Escolha uma pasta em uma unidade local: {folder}",
    ),
    (
        "watch.forbidden_folder",
        "Esta pasta não pode ser usada para monitoramento ou saída: {folder}",
    ),
    (
        "watch.bad_ext",
        "Tipo de arquivo desconhecido em uma regra de monitoramento: {ext}",
    ),
    (
        "watch.bad_output",
        "Uma regra de monitoramento tem um local de saída inválido",
    ),
    ("watch.missing_folder", "A pasta não existe: {folder}"),
    (
        "watch.start_failed",
        "Não foi possível monitorar a pasta: {err}",
    ),
    ("watch.label", "Monitorar: {folder}"),
    (
        "cli.no_ffmpeg",
        "Esta conversão precisa do ffmpeg, que não está instalado. Inicie o aplicativo uma vez para que ele o instale.",
    ),
    (
        "cli.no_office",
        "Documentos do Office precisam do suporte a documentos, que não foi baixado. Baixe-o no aplicativo em Configurações > Avançado ou execute `tessera --install-office`.",
    ),
    ("tray.open", "Configurações"),
    ("tray.quit", "Sair"),
    ("notify.dl_title", "Baixando o ffmpeg"),
    (
        "notify.dl_body",
        "A conversão de vídeo e MP3/OGG precisa do ffmpeg, uma ferramenta gratuita e de código aberto. Ele será baixado uma única vez (cerca de 170 MB).",
    ),
    ("notify.ready_title", "ffmpeg pronto"),
    (
        "notify.ready_body",
        "A conversão de vídeo e áudio está pronta para uso.",
    ),
    ("notify.done_title", "Convertido"),
    ("notify.failed_title", "Falha na conversão"),
    (
        "notify.target_off",
        "{fmt} está desativado nas configurações",
    ),
    (
        "menu.folder_images_to",
        "Converter todas as imagens em {fmt}",
    ),
    ("menu.folder_audio_to", "Converter todo o áudio em {fmt}"),
    (
        "menu.folder_docs_to",
        "Converter todos os documentos em {fmt}",
    ),
    ("menu.folder_preset", "{name} (todas as imagens)"),
    (
        "notify.folder_empty",
        "Nenhum arquivo para converter nesta pasta",
    ),
    (
        "notify.folder_cap",
        "Só os primeiros {max} arquivos desta pasta são convertidos",
    ),
    ("notify.folder_bad", "Essa pasta não pode ser usada"),
    (
        "drop.unsupported",
        "Nenhum desses arquivos pode ser convertido",
    ),
    (
        "drop.bad_choice",
        "Essa opção não serve para os arquivos selecionados",
    ),
    ("drop.custom_name", "Configurações personalizadas"),
    ("menu.custom", "Configurações personalizadas..."),
    (
        "cmd.autostart",
        "Não foi possível alterar a configuração de inicialização: {err}",
    ),
    (
        "cmd.menu_save",
        "Não foi possível salvar o menu de contexto: {err}",
    ),
    (
        "cmd.menu_remove",
        "Não foi possível remover o menu de contexto: {err}",
    ),
    (
        "cmd.save",
        "Não foi possível salvar as configurações: {err}",
    ),
    (
        "cmd.export",
        "Não foi possível exportar as configurações: {err}",
    ),
    (
        "cmd.import",
        "Não foi possível importar as configurações: {err}",
    ),
    ("cmd.bad_corner", "Canto inválido"),
    ("cmd.bad_seconds", "Duração inválida"),
    ("cmd.bad_path", "Caminho inválido"),
    ("cmd.no_path", "Caminho não encontrado"),
    ("job.done", "Concluído"),
    ("job.cancelled", "Cancelado"),
    (
        "job.needs_ffmpeg",
        "Esta conversão precisa do ffmpeg, que ainda não está pronto",
    ),
    ("panel.demo_name", "foto-exemplo.png"),
    (
        "tpl.length",
        "O nome do arquivo deve ter de 1 a {max} caracteres",
    ),
    (
        "tpl.unknown_token",
        "O nome do arquivo tem uma marca desconhecida; só {name}, {format} e {date} são permitidas",
    ),
    (
        "tpl.bad_char",
        "O nome do arquivo contém um caractere inválido: {c}",
    ),
    (
        "tpl.trailing",
        "O nome do arquivo não pode terminar com ponto ou espaço",
    ),
    (
        "tpl.reserved",
        "O nome do arquivo seria um nome reservado no Windows",
    ),
    ("ce.invalid_path", "Caminho de arquivo inválido: {detail}"),
    ("ce.not_found", "Arquivo não encontrado"),
    ("ce.access_denied", "Acesso negado"),
    ("ce.disk_full", "Disco cheio"),
    (
        "ce.corrupt",
        "O arquivo está corrompido ou ilegível. Tente abri-lo primeiro em outro programa.",
    ),
    (
        "ce.unsupported",
        "Este arquivo não pode ser convertido nesse formato. Escolha outro formato.",
    ),
    ("ce.trim_outside", "O corte começa depois do fim do clipe."),
    ("ce.too_large", "Arquivo grande demais"),
    (
        "ce.target_size",
        "Não é possível atingir o limite de tamanho sem deixar o resultado inutilizável. Tente um limite maior.",
    ),
    ("ce.protected", "O arquivo é protegido por senha"),
    (
        "ce.needs_ffmpeg",
        "Esta conversão precisa do ffmpeg. Instale-o em Configurações > Avançado.",
    ),
    (
        "ce.bad_pages",
        "Verifique os números das páginas: este PDF tem {n} páginas. Escreva-os como 1-3, 5.",
    ),
    (
        "ce.needs_office",
        "Documentos do Office precisam do suporte a documentos. Baixe-o em Configurações > Avançado.",
    ),
    ("ce.cancelled", "Cancelado"),
    (
        "ce.encode",
        "Falha na conversão: {detail}. Tente outro formato de destino.",
    ),
    ("ce.io", "Erro de arquivo: {detail}"),
    ("ce.not_file", "não é um arquivo"),
    ("ce.not_dir", "não é uma pasta"),
    ("ce.network_path", "caminho de rede ou de dispositivo"),
    ("ce.bad_name", "nome de arquivo inválido"),
    ("ce.no_free_name", "nenhum nome de arquivo livre"),
    (
        "ce.no_stream",
        "O arquivo não tem a faixa de áudio ou vídeo de que esta conversão precisa (por exemplo, sem som para MP3)",
    ),
    ("ce.empty_output", "Saída vazia"),
    ("ce.unknown", "erro desconhecido"),
    (
        "ce.ffmpeg_start",
        "Não foi possível iniciar o ffmpeg: {detail}",
    ),
    (
        "ce.ffmpeg_wait",
        "Não foi possível aguardar o ffmpeg: {detail}",
    ),
    (
        "ce.recycle",
        "Não foi possível mover o arquivo original para a Lixeira: {detail}",
    ),
    ("ff.read", "Não foi possível ler o arquivo: {detail}"),
    (
        "ff.hash",
        "Não foi possível verificar o arquivo baixado (SHA-256 não confere)",
    ),
    ("ff.start", "Não foi possível iniciar o download: {detail}"),
    (
        "ff.too_big",
        "O arquivo a ser baixado é maior que o esperado",
    ),
    (
        "ff.tmp",
        "Não foi possível criar um arquivo temporário: {detail}",
    ),
    ("ff.cut", "O download foi interrompido: {detail}"),
    ("ff.write", "Não foi possível gravar no disco: {detail}"),
    (
        "ff.archive",
        "Não foi possível abrir o arquivo compactado: {detail}",
    ),
    (
        "ff.unsafe",
        "O arquivo compactado contém um caminho de arquivo não seguro",
    ),
    (
        "ff.no_exe",
        "ffmpeg.exe não foi encontrado no arquivo compactado",
    ),
    ("ff.exe_big", "ffmpeg.exe é maior que o esperado"),
    ("ff.mkdir", "Não foi possível criar a pasta: {detail}"),
    ("ff.run", "Não foi possível executar o ffmpeg"),
    (
        "lo.unpack",
        "Não foi possível descompactar o suporte a documentos: {detail}",
    ),
    (
        "lo.missing",
        "O pacote de suporte a documentos está incompleto",
    ),
    (
        "ce.bad_pages_one",
        "Verifique os números das páginas: este PDF tem só uma página.",
    ),
    (
        "ce.no_pages_left",
        "Isso removeria todas as páginas. Mantenha pelo menos uma.",
    ),
    ("ce.empty_file", "O arquivo está vazio"),
    (
        "ce.office_timeout",
        "O LibreOffice não terminou em 5 minutos. O documento provavelmente é grande demais.",
    ),
    (
        "ce.no_recycle",
        "O original foi mantido: esta unidade não tem Lixeira",
    ),
    ("ce.exists", "Já existe um arquivo com esse nome"),
    (
        "preset.kind_target",
        "O formato escolhido não serve para todos os tipos de arquivo desta predefinição.",
    ),
    (
        "preset.dup_name",
        "Você já tem uma predefinição com este nome.",
    ),
    (
        "notify.preset_skip_one",
        "1 arquivo ignorado: sem suporte em {name}",
    ),
];
