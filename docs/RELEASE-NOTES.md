# 0.1.0 — Linux experimental

Primeiro pacote nativo Linux x86_64. Interface com biblioteca de vozes, soundboard,
laboratório de efeitos, gravador e configurações; núcleo local sem conta ou internet.

## Entrega

- Captura PipeWire, microfone virtual e monitoração independente, desligada por padrão.
- Efeitos tradicionais, pitch/formante, chorus, redução de ruído e 12 presets próprios.
- Importação WAV/MP3/Ogg Vorbis/FLAC, painéis, loop, sobreposição e pré-escuta.
- Biblioteca SQLite, presets JSON, gravação e replay de 30 segundos.
- Captura estéreo do desktop no Gravador, saída explícita e replay próprio de 30 segundos; funciona sem microfone.
- CLI, MCP e processamento de arquivos usando o mesmo registro de comandos.
- Interface revista e aprovada pelo usuário, com avatares originais e sem slogans.
- Correção de “Ouvir-me” em um único lado: monitoração e pré-escuta enviam o sinal
  aos dois canais dos fones; regressão validada em uma saída estéreo PipeWire privada.
- Automático usa os dispositivos padrão do desktop; a última intenção de conexão é lembrada.
  O motor tenta reconectar quando os dispositivos/servidor voltam; Desconectar cancela a retomada.
- Volume de Ouvir-me de 0 a 200% na barra principal, independente e salvo entre sessões.
- Laboratório com 11 blocos reordenáveis, ordem salva em presets/sessão e transição curta
  durante mudanças ao vivo. Presets antigos continuam abrindo com a ordem padrão.
- Biblioteca com filtro **Vozes originais** e imagem própria nos presets criados/importados.
  PNG/JPEG/WebP ficam como miniaturas locais, com opções de trocar ou remover.
- Laboratório com intensidade por efeito nas vozes A/B e mistura de dois presets com níveis
  independentes. Misturas e intensidades são salvas no preset; presets anteriores continuam abrindo.
- Validadores reproduzíveis de fluxo completo, atraso no grafo privado, captura pela libobs e
  estabilidade com memória limitada. Scripts e instruções em `docs/VALIDATION.md` acompanham o pacote.

Descompacte o arquivo de `dist/` e execute `./voice-app`. A primeira sessão começa parada;
as seguintes lembram se o áudio estava conectado. O pacote requer as bibliotecas de áudio
e de janela do sistema; consulte `RUNNING.txt`.

## Limitações conhecidas

- Áudio validado em PipeWire privado com sinais sintéticos e fonte real PulseAudio da libobs.
  Hardware, Discord, fluxo gráfico OBS, Wayland e sessões de 60 minutos/quatro horas ainda
  precisam de evidências específicas. O teste libobs não inclui cenas, encoder ou gravação OBS.
- Pitch/formante adicionam 88 ms de buffering algorítmico; redução de ruído, 20 ms.
  Combinados são 108 ms, antes dos buffers do sistema. A meta de 80 ms com pitch está aberta.
  Em 24 eventos por perfil no grafo privado, p95 de onset: básico 10 ms, pitch 99 ms e mistura
  101 ms. Esses valores não medem latência física ou transporte de chamada.
- O compressor passa a medir o sinal na própria posição da cadeia. Presets antigos com
  filtros e compressão podem mudar de nível/timbre; não há promessa de áudio idêntico.
- Pitch/formante compartilham um bloco, assim como os dois filtros. O teto final é fixo.
- Misturas são limitadas a duas cadeias simples. O atraso acompanha a cadeia ativa mais lenta;
  mudar a topologia aplica uma transição curta. Miniaturas são locais, separadas do JSON de voz.
- Atalhos globais, bandeja,
  conversão neural, Windows/macOS, comunidade e controle móvel permanecem planejados.
- Dispositivos fixos ausentes não são substituídos por outro mic/fone. O modo Automático
  acompanha os padrões do desktop; sem padrão elegível, escolhe um dispositivo disponível.
- A captura do desktop inclui todos os sons da saída (chamadas e Ouvir-me inclusive), sem exclusão
  por aplicativo. Começa desligada e exige nova ativação após perder a conexão; nunca alimenta
  automaticamente o microfone virtual. Arquivos longos vão ao disco; filas/replays são limitados.
  Overruns são mostrados no Gravador e podem causar cortes. Há uma única gravação por sessão.
- Distribuição inicial em `.tar.gz`, sem instalador, AppImage ou Flatpak.

Nenhum marco completo foi promovido a estável. Evidências e comandos de validação estão
em [DEVLOG.md](DEVLOG.md); o escopo por capacidade está em [FEATURES.md](FEATURES.md).
