# Voice App v0.1.0-alpha.6 — Linux experimental

O Voice App agora oferece atalhos globais e integração com a bandeja no KDE
Plasma em Wayland. Os atalhos selecionam uma voz, tocam um som do Soundboard,
param todos os sons e alternam o mute do microfone. Em **Configurações**, a opção
**Recolher na bandeja** mantém o áudio e os atalhos ativos. Clique no ícone para
reabrir a janela ou escolha **Sair** no menu da bandeja.

## Instalação

1. Baixe o arquivo `.tar.gz` Linux x86_64 e o `SHA256SUMS.txt` no mesmo
   diretório. Verifique o download com `sha256sum -c SHA256SUMS.txt`.
2. Extraia o pacote e consulte `RUNNING.txt` para as dependências. Para adicionar
   o Voice App ao menu do KDE, execute `bash scripts/install-desktop-shortcut.sh`
   no diretório extraído e abra o app pelo menu. Esse atalho usa um processo
   com limite de 3 GiB e swap desativado.
3. Em **Configurações → Atalhos globais**, escolha uma voz e um som, clique
   em **Ativar atalhos** e defina as combinações no KDE. É possível rever as
   teclas em **Configurações do Sistema → Atalhos**. O recurso usa o portal
   GlobalShortcuts do KDE.

O catálogo inclui 30 vozes, com 12 presets embutidos e 18 receitas aprovadas
com retratos. O núcleo offline oferece microfone virtual PipeWire, vocoder,
dois modos granulares, Soundboard, gravação e replay.

## Correções da revisão

- O Soundboard descarta respostas do painel anterior e volta a pedir a lista
  se a fila de comandos estiver cheia. O controle de ganho em **Vozes** mostra
  a faixa aplicada pelo motor.
- A fala capturada durante o mute fica fora dos efeitos com atraso. O
  Soundboard continua disponível enquanto o microfone está silenciado.
- A importação recusa pipes nomeados. A escrita de WAV exige um destino novo
  e recusa links simbólicos no nome de saída. A gravação finaliza no limite
  do formato RIFF e preserva o replay.
- A restauração da janela pela bandeja passou a tratar eventos de ponteiro
  Wayland que chegam depois da remoção do dispositivo. O launcher funciona
  também em caminhos com espaços e `%`.
- O AccessKit usa zbus 6.2.1. Os diálogos de arquivos usam rfd 0.17.2, com
  portal do desktop e alternativa via Zenity.

## Validação

A revisão local passou em 189 testes Rust, 52 testes Python, 1.141 testes
AT-SPI, Clippy, política de dependências e sete benchmarks de DSP. A
acessibilidade foi exercitada em X11 e Wayland. O pacote extraído abriu as
cinco telas, e o GIO abriu o launcher em um caminho com espaços e `%`.

Atalhos, mute e bandeja passaram em três sessões KDE isoladas, com KWin real,
teclas e áudio sintéticos e host de bandeja simulado. O maior pico de memória
dos ensaios gráficos foi de 232,85 MiB, com limite de 3 GiB e swap desativado.
O PipeWire isolado validou a saída virtual, a pré-escuta, a captura do desktop,
a gravação e o replay. A fonte de captura da libobs recebeu áudio.

A release permanece experimental. Pitch e mistura mediram 100 ms no p95 no
grafo de software, acima da meta de 80 ms. A latência física e a aceitação
com hardware e sessões prolongadas seguem abertas.
