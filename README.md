# voice-app

[Releases](https://github.com/aaronmlf/voice-app/releases) ·
[Contribuir](CONTRIBUTING.md) · [Reportar um problema](https://github.com/aaronmlf/voice-app/issues/new/choose)

Um estúdio de voz local e de código aberto, escrito em Rust. Vozes tradicionais, soundboard,
laboratório de efeitos, gravação e replay, com um microfone virtual PipeWire para outros aplicativos.

**Versão experimental Linux.** O percurso de áudio foi testado em um daemon PipeWire isolado;
captura pelo módulo de áudio da libobs também foi verificada. Discord, fluxo gráfico da OBS,
hardware real, sessões longas e outras distribuições ainda exigem evidências específicas.
Este é um projeto independente, sem vínculo com o Voicemod.

## Executar no Linux

Baixe o `.tar.gz` e `SHA256SUMS.txt` na página de
[releases](https://github.com/aaronmlf/voice-app/releases), confira `sha256sum -c SHA256SUMS.txt`,
descompacte e execute `./voice-app` na pasta extraída. As primeiras releases são pré-releases.
O arquivo `RUNNING.txt` descreve as dependências de execução; não é necessário compilar o pacote.
O workflow de release compila no Ubuntu 24.04; isso não substitui validação com hardware nessa
distribuição. Compilações locais podem exigir versões mais novas da glibc e das bibliotecas.

Para compilar a partir do código:

Requisitos: Linux x86_64 com PipeWire e um gerenciador de sessão (por exemplo, WirePlumber),
Rust 1.98+, compilador C/C++, Clang, pkg-config, bibliotecas de desenvolvimento do PipeWire,
X11/Wayland e OpenGL. Seletores de arquivos usam o portal do desktop.

```bash
# Ubuntu: dependências de compilação
sudo apt install build-essential clang libclang-dev pkg-config libpipewire-0.3-dev libxkbcommon-dev libwayland-dev libx11-dev libegl1-mesa-dev

bash scripts/cargo-low-memory.sh run --release -p voice-app
```

Na primeira abertura, clique em **Conectar áudio**: o modo Automático usa o microfone e a saída
padrão do desktop. Configurações permite escolher dispositivos fixos. A conexão é lembrada entre
aberturas e retoma quando os dispositivos voltam; **Desconectar** desativa essa retomada.
A monitoração começa desligada; ative **Ouvir-me** e ajuste seu volume de 0 a 200% na barra inferior.
O volume é salvo e não altera o microfone virtual nem o soundboard. Selecione **Voice App Virtual
Microphone** como entrada no Discord, OBS ou jogo. O microfone padrão do sistema não é alterado.
Fechar a interface encerra o motor quando ela o iniciou; uma sessão `voice-cli serve` externa
continua até `shutdown` ou Ctrl+C.

## Windows experimental

A portabilidade nativa usa WASAPI para preparar captura, efeitos, Ouvir-me estéreo,
pré-escuta e gravação, com o mesmo motor/biblioteca. A CI compila com MSVC em Windows
Server 2022; uma verificação adicional pelo Wine usa um prefixo isolado e áudio sintético.
Os resultados desses gates são registrados separadamente de testes em Windows 10/11 real.

A implementação experimental mais recente permite enviar efeitos e soundboard a um
**VB-CABLE instalado separadamente**. Em Configurações, escolha explicitamente **CABLE Input**
como saída virtual. No Discord ou Audacity/Tenacity, escolha **CABLE Output** como entrada.
**Ouvir-me**, seu volume e a pré-escuta continuam nos fones físicos selecionados.

Baixe e instale o cabo pelo [site oficial da VB-Audio](https://vb-audio.com/Cable/), conforme
os termos do fornecedor. O Voice App não inclui o cabo nem instala drivers; os dispositivos
padrão do Windows são preservados. Essa rota dispensa desenvolver e assinar um driver próprio.
Essa frente foi cancelada e o [PR #5](https://github.com/aaronmlf/voice-app/pull/5) foi fechado sem merge.

Isso descreve o código experimental, sem anunciar uma nova release. O percurso com VB-CABLE
instalado, efeitos e soundboard nos aplicativos de destino ainda precisa de validação em Windows.
Wine e os testes offline não comprovam essa etapa. Veja o [guia e as verificações pendentes](docs/WINDOWS.md).

## Capturar áudio do desktop

No **Gravador → Áudio do desktop**, escolha explicitamente a saída que recebe os sons e clique em
**Ativar captura**. O indicador mostra o nível; **Iniciar gravação** salva WAV estéreo/48 kHz em disco,
e **Salvar replay** salva até os últimos 30 segundos. Funciona com o microfone desconectado.
**Parar captura** finaliza a gravação dessa fonte. A captura começa desligada a cada abertura;
após perder a saída ou o servidor, é necessário ativá-la novamente.

A captura contém todos os sons da saída escolhida, incluindo chamadas, pré-escuta e Ouvir-me.
Ela não é enviada ao microfone virtual. Para reutilizar um trecho, importe o WAV no soundboard;
a biblioteca converte essa cópia para mono. Só um arquivo pode ser gravado por vez.
As filas e os replays têm capacidade fixa; a duração da gravação não aumenta o histórico em RAM.

## Uso pela CLI

```bash
# Terminal 1: serviço local; primeira sessão parada, última conexão lembrada
cargo run -p voice-cli -- serve

# Terminal 2: diagnóstico, vozes e início explícito
cargo run -p voice-cli -- doctor
cargo run -p voice-cli -- devices
cargo run -p voice-cli -- voices
cargo run -p voice-cli -- start
cargo run -p voice-cli -- select builtin.radio
cargo run -p voice-cli -- monitor true
cargo run -p voice-cli -- status
cargo run -p voice-cli -- stop
```

Use `--help` para todos os comandos. `--data-dir` e `--socket` permitem sessões isoladas.
Os caminhos enviados pela CLI são relativos ao diretório do cliente, não ao processo do serviço.

Para remover um som, abra **⋮ → Remover do soundboard** no cartão ou use o menu do botão
direito. Confirme com **Remover**; Cancelar, × e Esc fecham sem alterar o painel. Pela CLI,
use `voice-cli clips` para encontrar o ID e `voice-cli remove-clip ID`. A remoção é salva na
biblioteca e interrompe somente esse som; os arquivos de áudio são mantidos.

### Testar sem microfone

```bash
cargo run -p voice-cli -- tone /tmp/entrada-voice.wav
cargo run -p voice-cli -- process /tmp/entrada-voice.wav /tmp/radio-voice.wav --preset builtin.radio
cargo xtask bench
```

Esses comandos não abrem dispositivos nem reproduzem o som. O benchmark mede somente tempo
de execução do DSP; não mede latência ponta a ponta. Com o serviço ativo, `voice-cli mcp`
oferece `voice_status` e `voice_command` por MCP/stdio usando o mesmo registro da GUI e CLI.

## Funcionalidades

- Interface nativa em português: Vozes, Soundboard, Laboratório, Gravador e Configurações.
- Microfone virtual, captura e monitoração independentes; pré-escuta apenas nos fones.
- Gate, ganho, filtros, compressor, saturação, robô, eco, reverb e chorus.
- Pitch e formantes via Signalsmith, redução de ruído RNNoise e 12 presets próprios.
- Cadeia de 11 blocos reordenáveis no Laboratório, salva com o preset e com a sessão.
- Intensidade por efeito e mistura de duas vozes, com níveis independentes e atrasos alinhados.
- Filtros para vozes originais e próprias; imagens locais personalizadas nos presets próprios.
- Favoritos e biblioteca visual com avatares originais.
- Biblioteca SQLite; importação WAV/MP3/Ogg Vorbis/FLAC normalizada para mono 48 kHz.
- Painéis, sobreposição, loop, parar todos os sons e cortes que preservam o original.
- Presets JSON versionados; gravação WAV da voz processada ou do desktop estéreo, com replay de 30 segundos por fonte.
- Socket local privado, CLI e MCP; renderização offline e diagnósticos.

Veja [a matriz de funcionalidades](docs/FEATURES.md) para distinguir capacidades testadas,
experimentais e planejadas. No Laboratório, use as setas para processar um efeito antes/depois;
“Ordem padrão” restaura a sequência sem mudar os parâmetros. O teto de segurança fica ao final.
Veja [como personalizar vozes](docs/VOICE-LAB.md) para imagens, mistura e intensidade dos efeitos.
IA de conversão de voz, atalhos globais por
portal, bandeja do sistema, Windows/macOS, comunidade e celular ainda não estão entregues.

## Desenvolvimento e validação

Neste ambiente de 16 GiB, execute uma tarefa por vez. Cargo está limitado a um job, release não
usa LTO e o cache fica em `target/` no disco. Não use `/tmp` como diretório de build: aqui ele usa
RAM. Para limitar também os processos filhos a **3 GiB**, use:

```bash
bash scripts/cargo-low-memory.sh xtask check
bash scripts/cargo-low-memory.sh xtask bench
bash scripts/cargo-low-memory.sh xtask audio-check
bash scripts/cargo-low-memory.sh xtask package
```

O wrapper exige systemd de usuário e não inicia um build sem o limite. Em outras plataformas,
os comandos diretos abaixo continuam disponíveis, com `jobs = 1` por padrão.

```bash
cargo xtask check
cargo xtask corpus
cargo xtask bench
cargo xtask license-check
bash scripts/test-pipewire.sh
cargo xtask package
```

O smoke de PipeWire cria um daemon privado, injeta áudio sintético e verifica captura, DSP,
saída virtual, monitor, pré-escuta e limpeza de nós. Não modifica a sessão de áudio do usuário.
Os testes de áudio usam sinais/arquivos sintéticos próprios, sem gravações pessoais.
Os validadores de fluxo, latência por software, libobs e sessões prolongadas usam o binário
pronto. Consulte [os procedimentos e limites](docs/VALIDATION.md). O wrapper Cargo e os
validadores compartilham um lock no workspace que recusa uma segunda tarefa pesada.

O pacote `.tar.gz` em `dist/` contém binários, documentação, licenças e avisos de terceiros.
Não instala serviços, drivers nem altera configurações globais. Consulte
[arquitetura](docs/ARCHITECTURE.md), [roteiro](docs/ROADMAP.md), [protocolo](docs/CONTROL.md),
[resultados](docs/DEVLOG.md), [notas da versão](docs/RELEASE-NOTES.md) e
[contrato dos agentes](AGENTS.md).
O procedimento de publicação está em [RELEASING.md](docs/RELEASING.md).

## Dados e licenças

A biblioteca fica em `$XDG_DATA_HOME/voice-app` ou `~/.local/share/voice-app`; o socket fica em
`$XDG_RUNTIME_DIR/voice-app/control.sock`. Configurações e áudios permanecem locais.
Não há login, telemetria, download automático de modelos nem envio de microfone à internet.
Para remover os dados, feche todas as sessões e remova explicitamente o diretório da biblioteca.

Código próprio: **MIT OR Apache-2.0**. Dependências e fontes mantêm seus termos, incluindo MPL-2.0
para Symphonia. O empacotamento inclui os respectivos avisos e fontes MPL inalteradas; o relatório
SPDX não substitui uma auditoria de vulnerabilidades. Nenhum modelo de voz proprietário é distribuído.

