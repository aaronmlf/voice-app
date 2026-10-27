# Voice App v0.1.0-alpha.3 — Linux experimental

Esta versão Linux x86_64 acrescenta vocoder tradicional de 20 bandas e granular
nos modos Estilizado e Natural experimental, com intensidade por efeito nas vozes
A/B. O Laboratório passa a ter 13 blocos reordenáveis, usando o mesmo contrato
GUI/CLI/MCP. Ordens/intensidades antigas de 11/12 posições são migradas preservando
a ordem relativa. Os novos efeitos começam desligados nas vozes existentes.

O pacote contém somente os **12 presets embutidos originais**, com receitas
preservadas. Presets, imagens, biblioteca, gravações e comparações pessoais não
acompanham a distribuição. O núcleo continua offline, sem login.

Permanecem disponíveis o microfone virtual PipeWire, soundboard após o DSP com
remoção de sons, Ouvir-me estéreo com volume próprio, pré-escuta separada,
gravação/replay de 30 segundos e captura explícita do desktop. O aplicativo não
muda o microfone padrão; monitoração começa desligada em uma biblioteca nova.

## Instalação e atualização

Confira o SHA-256 em `SHA256SUMS.txt`, descompacte o arquivo Linux e execute
`./voice-app`. Consulte `RUNNING.txt` para as dependências. Não é um instalador.
Faça backup da biblioteca/presets antes de atualizar: o formato ganha campos e
posições de efeitos; voltar a versões anteriores não é qualificado.

## Evidências e limitações

A [CI integrada anterior](https://github.com/aaronmlf/voice-app/actions/runs/40526368003)
passou 198 testes Rust, 52 Python, formatação, Clippy, limites entre componentes,
corpus, sete benchmarks, política de dependências/licenças, PipeWire privado e
pacote. A publicação repete os gates no commit exato da release.

- Vocoder e ambos os modos granular chegaram ao consumidor virtual no grafo
  privado; soundboard pós-DSP e pré-escuta separada foram verificados. Hardware,
  fala percebida e os novos efeitos no Discord/Audacity/Tenacity ainda precisam
  de aceitação específica.
- Natural limita pitch efetivo a ±4 semitons e grãos a 40 ms. Seu nome descreve
  o objetivo; naturalidade/clareza percebidas precisam de audição. O buffering
  padrão a 48 kHz é 962 frames/20,04 ms, sem medir latência ponta a ponta.
- Pitch/formante acrescentam 88 ms e denoise 20 ms de buffering algorítmico.
  Combinações podem exceder a meta física de 80 ms, que continua aberta.
- Quatro grãos e buffers são preparados fora do callback, com trabalho limitado.
  Não há novo passe prolongado de memória/estabilidade. A tentativa anterior de
  quatro horas falhou e foi encerrada; o passe básico histórico de 60 minutos
  não qualifica automaticamente este binário.
- Captura do desktop inclui os sons da saída selecionada, chamadas e Ouvir-me;
  não há exclusão por aplicativo. Nunca alimenta automaticamente o mic virtual.
- Atalhos globais Wayland, IA neural, macOS, comunidade e controle móvel continuam
  sem qualificação. Esta release é experimental e não foi promovida a beta.
- Três exceções de manutenção de dependências expiram em 22/11/2026 e exigem revisão
  na CI. Consulte `SECURITY.md`.

Veja `docs/VALIDATION.md`, `docs/VOCODER.md`, `docs/GRANULAR.md` e
`docs/FEATURES.md` para evidências e escopo. Retenha as licenças, avisos e fontes
de terceiros incluídos ao redistribuir.
