# Atalho Linux do Voice App

`bash scripts/install-desktop-shortcut.sh` instala três arquivos pequenos no prefixo padrão
`$HOME/.local`: `bin/voice-app`, `share/applications/voice-app.desktop` e
`share/icons/hicolor/scalable/apps/voice-app.svg`. `--prefix PATH` permite inspecionar a instalação
em um diretório isolado. O launcher aponta para o binário já pronto em `target/release/voice-app`
do checkout atual; builds futuros nesse caminho passam a ser usados automaticamente.

A identidade `voice-app` corresponde ao app_id Wayland atribuído por eframe ao
`run_native("voice-app")`. No X11, o winit usa o nome do binário como WM_CLASS. Não exige
recompilar o aplicativo. A entrada
fica na categoria Multimídia/Áudio. No KDE, procure **Voice App** e use o menu de contexto para
adicionar aos favoritos ou ao painel. O instalador não altera esses itens por conta própria.

O launcher cria uma unidade transitória de usuário `voice-app.service`, com MemoryHigh=2500M,
MemoryMax=3G e MemorySwapMax=0. Reaproveita o ambiente gráfico do clique, não compila, não copia o
binário, não registra inicialização automática e não altera o microfone padrão. Não cria outra
janela se `voice-app.service` ou a sessão de desenvolvimento `voice-app-preview.service` já estiver
ativa. O gerenciador de tarefas do KDE associa a janela ao launcher pelo app_id. Um clique no menu
com uma sessão já ativa apenas evita a duplicação; a ativação dessa janela pelo menu não está
implementada. Use o item correspondente no painel/Alt+Tab para trazê-la à frente.

Para atualizar o cache do menu no KDE: `kbuildsycoca6 --noincremental`. O instalador permite que
o integrador faça essa chamada no momento apropriado. Nenhum comando de painel/favoritos é executado.

Validação de sintaxe: `bash -n scripts/install-desktop-shortcut.sh` e
`desktop-file-validate <prefix>/share/applications/voice-app.desktop`. A checagem estática não é
uma prova visual de pinagem em todas as versões do Plasma.

Nesta máquina KDE/Wayland, o atalho foi instalado e o cache atualizado. A abertura por `gio launch`
e um segundo clique foram verificados: serviço ativo, mesmo PID e limite de 3 GiB aplicado.
Evidência local em `target/desktop-launcher-qa/runtime-report.json`. A fixação no painel/favoritos
continua sendo uma ação manual do usuário.

O SVG é um desenho vetorial original licenciado MIT OR Apache-2.0, como o código do projeto.
