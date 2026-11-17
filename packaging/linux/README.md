# Atalho Linux do Voice App

`bash scripts/install-desktop-shortcut.sh` instala três arquivos pequenos no prefixo padrão
`$HOME/.local`: `bin/voice-app`, `share/applications/voice-app.desktop` e
`share/icons/hicolor/scalable/apps/voice-app.svg`. `--prefix PATH` permite inspecionar a instalação
em um diretório isolado. O launcher aponta para o binário já pronto em `target/release/voice-app`
no checkout ou `./voice-app` dentro do pacote extraído. Mantenha o pacote em um local fixo.

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

## Dependências dos diálogos de arquivos

O rfd 0.17 usa libdbus e um backend de XDG Desktop Portal compatível com seleção
de arquivos (GTK, GNOME ou KDE). Zenity é o fallback quando o portal não está
disponível e também é usado para caixas de mensagem. O backend wlroots sozinho
não implementa a interface de seleção de arquivos. A distribuição deve fornecer
esses componentes; o aplicativo não os instala nem muda a configuração do desktop.

Abrir, salvar e cancelar devem ser validados em Wayland e X11 no ambiente de
distribuição. Uma compilação aprovada não substitui esses testes gráficos.

## Atalhos globais no KDE/Wayland

O binário Linux depende de `libdbus-1.so.3` (`libdbus-1-3` no Ubuntu).
Instale a entrada do menu com o script de atalho acima. Use
`xdg-desktop-portal` e `xdg-desktop-portal-kde` com a interface
GlobalShortcuts. Em Configurações → Atalhos globais, escolha a voz e o som
e ative os atalhos. As teclas são atribuídas no KDE.
O atalho de microfone alterna o mute da voz e permite usar o Soundboard.
Em Configurações, Recolher na bandeja mantém o áudio e os atalhos ativos.
No ícone do Voice App, use Abrir para restaurar a janela ou Sair para
encerrar o aplicativo.
