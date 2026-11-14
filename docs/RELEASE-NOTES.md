# Voice App v0.1.0-alpha.5 — Linux experimental

Esta versão amplia o catálogo e mantém as correções da
[issue #7](https://github.com/aaronmlf/voice-app/issues/7).

- O AccessKit mantém a integração com leitores de tela via AT-SPI e passa a
  usar zbus 6.2.1. Os sinais de cache da árvore de acessibilidade seguem as
  assinaturas AT-SPI. O seletor de arquivos rfd 0.17.2 usa o portal do desktop
  com alternativa via Zenity.
- A política de dependências agora proíbe zbus anterior à versão 6 e os três
  crates descontinuados. As exceções temporárias foram removidas e as
  verificações de avisos, licenças e origens continuam ativas.
- O SQLite embutido foi atualizado para 3.54.1 e rustix para 1.1.6. Os testes
  abriram uma biblioteca sintética no esquema anterior.

O pacote inclui **30 vozes**. Os 12 presets embutidos são acompanhados por
18 receitas com retratos. Use **Imagem** no cartão de qualquer voz para
trocar ou restaurar o retrato. O núcleo funciona offline e inclui vocoder,
dois modos granulares, microfone virtual PipeWire, soundboard e gravação.

## Instalação

Baixe o pacote Linux e o arquivo `SHA256SUMS.txt` no mesmo diretório. Confira
o download com `sha256sum -c SHA256SUMS.txt`, extraia o `.tar.gz` e execute
`./voice-app` no diretório extraído. Consulte `RUNNING.txt` para as dependências.
Guarde uma cópia da biblioteca e dos presets locais antes de atualizar.

## Evidências e limites

Os checks do [PR #10](https://github.com/aaronmlf/voice-app/pull/10) cobriram a
política de dependências, os sinais AT-SPI, a acessibilidade em X11, o seletor
de arquivos e o grafo PipeWire privado. Os testes locais verificaram anúncios
do Orca em X11, reativação da árvore em Wayland e abertura, salvamento e
cancelamento pelo seletor KDE em Wayland. A publicação repete os gates Linux
no commit que gera o pacote.

A release permanece experimental. A latência física, a sessão de quatro
horas e a aceitação em hardware com os efeitos recentes seguem pendentes.
Veja [VALIDATION.md](https://github.com/aaronmlf/voice-app/blob/v0.1.0-alpha.5/docs/VALIDATION.md)
para os cenários testados e seus limites.
