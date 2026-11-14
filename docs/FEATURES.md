# Matriz de funcionalidades

Os estados abaixo se referem às condições de teste descritas. O Voice App continua
experimental. A branch `beta` é um canal de integração, não uma qualificação do produto.

A pré-release [Linux alpha.4](https://github.com/aaronmlf/voice-app/releases/tag/v0.1.0-alpha.4)
inclui a migração de acessibilidade para zbus 6, o seletor rfd atualizado e as
correções de SQLite e rustix verificadas no
[PR #10](https://github.com/aaronmlf/voice-app/pull/10).
A [Windows alpha.4](https://github.com/aaronmlf/voice-app/releases/tag/windows-v0.1.0-alpha.4)
incorpora a correção de SQLite da mesma revisão. A CI Windows exercita a
biblioteca, os comandos offline e a interface com áudio desativado.
As pré-releases alpha.5 incluem os 12 presets embutidos e 18 vozes com
retratos. A biblioteca, as imagens substituídas e as gravações ficam locais.

## Núcleo e recursos compartilhados

| Capacidade | Estado | Evidência e limites |
|---|---|---|
| Contratos, atômicos e filas | Verificados em testes | Validação, concorrência, saturação e wrap de contador. Capacidade fixa; processamento em tempo real sem alocação Rust nos caminhos testados. |
| DSP tradicional, pitch/formante, chorus e redução de ruído | Verificados offline; ao vivo experimental | Testes espectrais, ruído, extremos e continuidade entre blocos. Pitch/formante acrescentam 88 ms e redução de ruído, 20 ms de buffering algorítmico a 48 kHz; não é latência física. |
| Laboratório de efeitos | Experimental; CI integrada aprovada | 13 blocos reordenáveis, intensidade por efeito em A/B e mistura de duas vozes. Migração das ordens/intensidades antigas preserva índices e ordem relativa, com os novos efeitos desligados nos presets existentes. |
| Vocoder tradicional | Experimental; testes offline e grafo privado aprovados | 20 bandas, portadora interna e proteção de consoantes. Smoke Windows do pacote extraído também passou. Audição e uso dos novos efeitos em aplicativos de destino ainda precisam de validação específica. Veja [VOCODER.md](VOCODER.md). |
| Granular Estilizado e Natural | Experimental; testes offline e grafo privado aprovados | Quatro grãos e buffers fixos; smoke Windows cobre os dois modos. No perfil Natural padrão, buffering de 20,04 ms a 48 kHz. Qualidade percebida e áudio físico continuam pendentes. Veja [GRANULAR.md](GRANULAR.md). |
| Biblioteca e imagens das vozes | Experimental | Catálogo de 30 vozes. A troca e a restauração de imagem estão disponíveis em qualquer preset. O SQLite conserva overrides PNG/JPEG/WebP, favoritos e os ajustes locais. |
| Soundboard | Experimental | Importação WAV/MP3/Ogg Vorbis/FLAC normalizada para mono/48 kHz; painéis, loop, sobreposição, recorte e parada. Remover som exclui o cadastro e interrompe somente aquele som; os arquivos de áudio permanecem. |
| Monitoração e pré-escuta | Experimentais | Rotas separadas da saída virtual, com volume independente de Ouvir-me (0–200%) e pré-escuta nos fones. Isolamento exercitado no PipeWire privado. |
| Gravação e replay | Experimentais | Gravação em WAV e replay dos últimos 30 s por fonte; captura do desktop explícita e separada da saída virtual. Apenas uma gravação de arquivo por vez. |
| GUI, CLI e MCP | Experimentais | Registro de comandos compartilhado. CLI/MCP e persistência verificados em testes; smokes de GUI não qualificam interação manual ou sessão longa. |

## Linux

| Capacidade | Estado | Evidência e limites |
|---|---|---|
| Acessibilidade da GUI via AT-SPI | Candidato validado em sessões isoladas | X11 e Wayland com libatspi verificaram árvore, nomes e papéis, foco, acionamento e eventos de cache. Orca 50.3 em X11 gerou os anúncios de **Criar voz** e **Importar** e a tecla Enter abriu o Laboratório. |
| Diálogos de arquivo Linux | Candidato validado em sessões isoladas | O probe verificou abertura, salvamento com destino explícito e cancelamento com Unicode no portal KDE em X11 e Wayland e no Zenity em X11. |
| Captura, fonte virtual e reconexão PipeWire | Experimentais | Cliente externo, módulo de captura da libobs e integração no PipeWire privado. Retorno do daemon, troca de padrão, reconexão e cancelamento por Stop exercitados em fixtures. Usuário relatou voz com efeito e soundboard no Discord desktop em sessão anterior; GUI da OBS e compatibilidade ampla permanecem abertas. |
| Gravação no Tenacity | Teste prático de uma sessão | Tenacity Flatpak, ALSA/`pulse`, mono a 48 kHz. A captura foi direcionada por aplicativo para Voice App Virtual Microphone, mantendo o microfone padrão. A nova gravação preservou a rota, e o usuário confirmou ter ouvido voz com os efeitos já ativos e os dois bipes do soundboard. Sem análise do WAV, medição de latência ou qualificação de vocoder/granular específicos. |
| Captura do desktop | Experimental | Sink explícito, áudio estéreo, WAV/replay independentes e isolamento da fonte virtual verificados no grafo privado. |
| Pacote Linux `.tar.gz` | Experimental | Build no Ubuntu 24.04 x86_64, checksum e conteúdo conferidos. Smokes de GUI em Xvfb e CLI no PipeWire privado; hardware, outras distribuições e fluxos longos são etapas separadas. |
| Entrada no menu Linux/KDE | Verificada em uma sessão | Entrada e ícone instalados; abertura do binário pronto e segundo clique sem duplicação observados. Pinagem manual pelo KDE. |
| Sessões prolongadas e latência | Qualificação parcial | Um perfil básico anterior passou 60 min em grafo privado. A tentativa de mistura por quatro horas falhou por novo `monitor_overruns`. As pré-releases atuais ainda precisam de validação prolongada e de latência física. Veja [VALIDATION.md](VALIDATION.md). |

## Windows

| Capacidade | Estado | Evidência e limites |
|---|---|---|
| Pacote nativo `.zip` | Experimental | CI MSVC em Windows Server 2022, checksum, inspeção PE, CLI/IPC/MCP e smoke de GUI com renderização por software e áudio desligado. Executáveis sem assinatura. Windows 10/11 real com áudio permanece pendente. |
| Rota externa VB-CABLE | Experimental | Requer instalação separada do VB-CABLE base x64. Em VM anterior, o Audacity recebeu tons e fala sintética via soundboard; houve underruns/overruns. Isso não qualifica voz física, Discord/Tenacity, reconexão nem estabilidade da release atual. Veja [WINDOWS.md](WINDOWS.md). |
| Captura do desktop WASAPI | Experimental | Loopback digital anterior em VM registrou WAV estéreo/48 kHz, com um overrun. Não qualifica audição física ou sessão longa. |
| Wine | Offline verificado; GUI/IPC não qualificados | Comandos offline passaram; a recusa segura quando o ambiente não oferece DACL protegida também foi verificada. Não equivale a compatibilidade Wine. |
| Driver próprio Windows | Cancelado | [PR #5](https://github.com/aaronmlf/voice-app/pull/5) fechado sem merge. O protótipo legado fica em `windows-native` e fora dos pacotes. A rota VB-CABLE não depende desse projeto. |

Atalhos globais, bandeja, conversão neural local, macOS, AppImage/Flatpak do Voice
App, comunidade e controle móvel continuam planejados.

Para os resultados completos e seu contexto, consulte [VALIDATION.md](VALIDATION.md)
e o [histórico público de desenvolvimento](DEVLOG.md). O
[roteiro](ROADMAP.md) separa critérios futuros de capacidades já exercitadas.
