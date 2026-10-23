# Matriz de funcionalidades

“Verificada” indica o contexto de teste descrito, não equivalência de qualidade com outro produto.
Nenhum marco inteiro foi promovido a estável.

| Capacidade | Estado | Evidência / pendência |
|---|---|---|
| Contratos, atômicos e filas limitadas | Verificada em testes | Ordem, concorrência, saturação, wrap de contador e validação |
| DSP tradicional | Verificada offline; ao vivo experimental | Sinais sintéticos, extremos, preservação entre blocos e ausência de alocação Rust |
| Fonte virtual + captura | Experimental | Cliente externo e módulo real `pulse_input_capture` da libobs 32.2.2 receberam sinal no grafo privado. Usuário relatou voz com efeito e soundboard funcionando no Discord desktop Linux em 22/10; gravação Audacity/Tenacity, GUI OBS e medições físicas pendentes |
| Monitoração e pré-escuta separadas | Experimental | Ganho, sinal idêntico em L/R e isolamento medidos em sink estéreo no daemon privado |
| Detecção e reconexão automática | Experimental | Metadata de padrões, troca durante a sessão, retorno do daemon, retomada entre processos e cancelamento por Stop testados em fixture privada |
| Volume de Ouvir-me | Experimental | Controle de 0–200% independente; valor e persistência verificados por CLI/motor |
| Interface nativa | Experimental; smoke nativo aprovado | [Windows Server 2022, `bd75106`](https://github.com/aaronmlf/voice-app/actions/runs/40008350538): GUI extraída abriu em WGPU/DX12/CPU, renderizou PNG 1024×768 igual a dez medições da área cliente, verificou áudio desligado e encerrou com código 0. Viewport limitado ao monitor único; interação manual, layout maior e sessão longa pendentes |
| Filtro de vozes originais | Verificada em GUI isolada | Clique na biblioteca mostrou somente os 12 presets builtin; Minhas vozes mostrou apenas vozes de usuário, sem trocar a voz selecionada. Presets pessoais ficam fora da distribuição pública |
| Imagens de presets próprios | Experimental | PNG/JPEG/WebP, recorte, limites, falhas e persistência verificados; imagem gerenciada após remover fonte e reiniciar, renderização circular e comandos CLI/MCP testados |
| SQLite e presets JSON | Verificada em testes | Persistência, versão, limites e dados inválidos |
| WAV/MP3/Ogg Vorbis/FLAC | Verificada offline | Fixtures próprias; mono48k, resampling, limites e corte |
| Painéis, loop e sobreposição | Experimental | Mixer e comandos testados; sessão prolongada do perfil de mistura reprovada por incremento de contador, sem qualificação de quatro horas |
| Remover som do soundboard | Experimental; testes aprovados | Menu ⋮ e botão direito, confirmação e comando único GUI/CLI/MCP. Persistência, IDs inválidos, isolamento dos outros sons e arquivos preservados cobertos pelo gate local de 111 testes Rust/51 Python. Remove o cadastro e para apenas esse som; arquivos originais e cópias normalizadas ficam em disco. CI pública do commit `bd75106`: Linux 111 testes Rust/51 Python e Windows 99 Rust/15 Python aprovados. Interação gráfica de remoção ainda pendente |
| Gravação processada e replay30s | Experimental | Worker/PCM, finalização e preservação; criar voz A/B→gravar→cortar→soundboard→reiniciar→reutilizar sem WAV externo aprovado por comandos no grafo privado |
| CLI e MCP | Verificada em testes locais | Mesmo registro; sessão parada, biblioteca e mensagens JSON-RPC |
| Pacote Linux `.tar.gz` | Experimental | Checksum validado; binários extraídos executados, GUI em Xvfb e CLI integrado ao PipeWire privado |
| Código público e releases GitHub | Experimental | `v0.1.0-alpha.1`, commit `046ae02`, publicada pelo Actions após 90 testes Rust, 40 Python, integração privada, corpus, benchmark e política de dependências. Dois assets reais com SHA-256 conferido e 23 arquivos documentais/scripts iguais ao snapshot público; binários desta release inspecionados por ELF, sem execução local durante o soak. Somente os 12 presets originais |
| Pitch/formante, chorus, denoise | Verificada offline; ao vivo experimental | Backend real, testes espectrais/ruído/zero alloc Rust; pitch88ms, denoise20ms, ambos108ms de buffering algorítmico |
| Reordenação de efeitos | Experimental | 11 blocos no Laboratório; validação de permutação, atualização atômica, diferença no áudio offline/PipeWire, persistência e importação de presets antigos testadas |
| Mistura de duas vozes | Experimental | Ramos alinhados, transições e persistência; áudio externo por 30 s sem novos erros; atraso de onset p95 101 ms no grafo privado, 24 eventos, perfil sem denoise; latência física pendente; tentativa de quatro horas interrompida por +1 `monitor_overruns` após 10.675,083 s saudáveis |
| Intensidade por efeito | Experimental | 11 intensidades em A e B; bypass, wet e interpolação, alinhamento dry/wet de pitch, presets antigos e ausência de alloc/free Rust em processamento verificados |
| Atalho no menu Linux/KDE | Verificado nesta sessão | Entrada desktop e ícone instalados; abertura do binário pronto por gio, identidade voice-app, limite de 3 GiB e segundo clique sem duplicação verificados; pinagem manual pelo menu do KDE |
| Captura do desktop | Experimental | Sink explícito, estéreo L/R, WAV/replay independentes, isolamento do virtual, finalização na perda do daemon e captura desligada após reiniciar verificados em PipeWire privado; hardware e sessões longas pendentes |
| Validação reprodutível de áudio | Verificada em grafo privado; quatro horas reprovadas | Básico de 60 min aprovado: 3.600,018 s ativos/716 janelas saudáveis. Mistura: 10.675,083 s saudáveis de 14.400 s exigidos, 2.128/2.129 janelas e +1 `monitor_overruns`; serviço terminou com código 1. RSS máximo do motor 23,30 MiB; pico do serviço 51,24 MiB, sem OOM. PCM descartado, replay limitado. Ver `VALIDATION.md` |
| Atalhos globais / bandeja | Planejada | Portal Wayland e fallback CLI |
| IA de conversão de voz | Planejada | Worker, modelos/licenças/hashes e hardware medido |
| Windows nativo | Experimental; CI nativa aprovada | [Commit bcb3f45](https://github.com/aaronmlf/voice-app/actions/runs/40037022519): 135 testes Rust/15 Python, quatro reexecuções de corpus e demais gates nativos aprovados. GUI extraída: 5,488 s, pico observado 75,12 MiB do processo, áudio desligado e PNG 1024×768 limitado ao monitor único. Não qualifica áudio instalado, interação manual, processo filho ou sessão longa; ver WINDOWS.md |
| Saída Windows por VB-CABLE externo | Experimental; CI nativa aprovada, aceitação instalada pendente | Seleção explícita por ID/par PnP; efeitos+soundboard no cabo, fones/Ouvir-me/pré-escuta separados, fila/scratch de 64 KiB reutilizados. [Windows bcb3f45](https://github.com/aaronmlf/voice-app/actions/runs/40037022519): 135 Rust/15 Python, corpus, benchmark, pacote, CLI/IPC/MCP e GUI sem áudio aprovados. Cabo instalado, Discord/Audacity/Tenacity, RSS/CPU ativos e latência real pendentes |
| Driver próprio Windows (histórico) | Cancelado; PR #5 fechado sem merge | Evidências antigas de compilação WDK/INF/C++ e testes da ponte foram preservadas; não houve assinatura, instalação ou qualificação do kernel. Esta frente não é necessária para a rota externa VB-CABLE e não tem trabalho ativo de assinatura |
| macOS | Planejada | Plugin virtual, assinatura e hardware próprios |
| Comunidade/mobile | Planejada | Serviços e pareamento ainda não implementados |
| AppImage/Flatpak | Planejada | Distribuição inicial em tar.gz |


Em [Linux/Ubuntu 24.04, commit `bd75106`](https://github.com/aaronmlf/voice-app/actions/runs/40008350530),
111 testes Rust e 51 Python, formatação, Clippy, limites, corpus, benchmark, política
de dependências/licenças, integração PipeWire privada e pacote foram aprovados. Isso
não acrescenta uma qualificação de quatro horas, hardware ou latência física.



Registro local da publicação em 23/10: [prévia Windows windows-v0.1.0-alpha.1](https://github.com/aaronmlf/voice-app/releases/tag/windows-v0.1.0-alpha.1)
publicada pelo workflow40015353388 a partir da fonte pública94da7b9. ZIP e checksum
baixados em streaming e conferidos; 19 arquivos próprios/notice iguais ao snapshot
revisado, ambos os PE AMD64 e hashes GUI/CLI correspondentes à evidência nativa.
Smoke de GUI em Windows Server2022:5,705s, pico75,35MiB, áudio desligado. Wine
verificou comandos offline e recusa de DACL, sem aprovação GUI/IPC. Isso não
qualifica driver virtual, dispositivos Windows10/11 físicos, chamadas, gravadores,
latência física ou beta. Quatro horas continuam deixadas de lado por pedido.


Atualização de 23/10/2026: publicada a pré-release experimental
[Windows windows-v0.1.0-alpha.2](https://github.com/aaronmlf/voice-app/releases/tag/windows-v0.1.0-alpha.2),
tag `bd6f6b4`, binários da CI `bcb3f45` e apenas 12 vozes originais. O job Wine
terminou: comandos offline e recusa segura de DACL verificados no Wine 9.0;
resultado `unsupported_security_environment`, sem qualificação de GUI, IPC ou
compatibilidade Wine. O VB-CABLE exige instalação separada e licença própria.
Cabo instalado, Discord/Audacity/Tenacity, hardware, Ouvir-me/pré-escuta nos fones,
RSS/CPU com áudio ativo, latência física e sessões prolongadas continuam pendentes.
O pico nativo de 75,12 MiB é apenas da GUI curta com áudio desligado. A meta de
80 ms continua aberta; não há promoção a beta nem retomada do driver próprio.
