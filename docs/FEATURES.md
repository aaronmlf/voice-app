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
| Coleções locais de vozes originais e próprias | Experimental; testes locais anteriores à integração | As 12 vozes builtin continuam originais; uma voz própria pode ser classificada explicitamente na biblioteca local sem mudar ID, DSP ou imagem. Salvar/importar preserva a curadoria existente e intercâmbio não transfere a classificação. Isso não autoriza publicação; a distribuição contém somente as 12 vozes builtin. CI integrada e interação manual pendentes |
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
| Reordenação de efeitos | Experimental | 13 blocos no Laboratório, incluindo vocoder e granular. Migração das ordens legadas de 11/12 posições preserva os índices históricos e a ordem relativa; validação, atualização atômica e persistência cobertas por testes locais anteriores à integração. CI integrada pendente |
| Mistura de duas vozes | Experimental | Ramos alinhados, transições e persistência; áudio externo por 30 s sem novos erros; atraso de onset p95 101 ms no grafo privado, 24 eventos, perfil sem denoise; latência física pendente; tentativa de quatro horas interrompida por +1 `monitor_overruns` após 10.675,083 s saudáveis |
| Intensidade por efeito | Experimental | 13 intensidades em A e B; arrays legados recebem valor 1 nos novos índices sem ligar vocoder/granular. Bypass, interpolação e alinhamento cobertos por testes locais anteriores à integração; CI integrada pendente |
| Vocoder tradicional | Experimental; DSP local e grafo privado testados antes da integração | Banco de 20 bandas, portadora interna, proteção de consoantes e controles GUI/CLI/MCP. Estado preparado, bypass e ausência de buffering de fluxo próprios; resposta temporal dos filtros permanece. Soundboard pós-DSP e pré-escuta separada exercitados em PipeWire privado. CI integrada, audição e testes dos novos efeitos nos aplicativos de destino pendentes; ver VOCODER.md |
| Granular Estilizado / Natural | Experimental; Natural requer avaliação perceptual | Quatro grãos e buffers fixos; limites, migração, transições, reset e processamento sem alloc/free Rust cobertos por testes locais anteriores à integração. Natural efetivo ±4 semitons/grão até 40 ms, sem dispersão/retorno. A 48 kHz, perfil Natural padrão anuncia 20,04 ms de buffering; não é latência física. CI integrada, áudio nativo e qualidade percebida pendentes; ver GRANULAR.md |
| Atalho no menu Linux/KDE | Verificado nesta sessão | Entrada desktop e ícone instalados; abertura do binário pronto por gio, identidade voice-app, limite de 3 GiB e segundo clique sem duplicação verificados; pinagem manual pelo menu do KDE |
| Captura do desktop | Experimental | Sink explícito, estéreo L/R, WAV/replay independentes, isolamento do virtual e recuperação verificados em PipeWire privado. No Windows, loopback digital dos fones gerou WAV estéreo/48 kHz/16 bits de 57,31 s; +1 overrun observado. Isso não comprova audição física ou sessão longa |
| Validação reprodutível de áudio | Verificada em grafo privado; quatro horas reprovadas | Básico de 60 min aprovado: 3.600,018 s ativos/716 janelas saudáveis. Mistura: 10.675,083 s saudáveis de 14.400 s exigidos, 2.128/2.129 janelas e +1 `monitor_overruns`; serviço terminou com código 1. RSS máximo do motor 23,30 MiB; pico do serviço 51,24 MiB, sem OOM. PCM descartado, replay limitado. Ver `VALIDATION.md` |
| Atalhos globais / bandeja | Planejada | Portal Wayland e fallback CLI |
| IA de conversão de voz | Planejada | Worker, modelos/licenças/hashes e hardware medido |
| Windows nativo | Experimental; CI nativa aprovada | [Commit bcb3f45](https://github.com/aaronmlf/voice-app/actions/runs/40037022519): 135 testes Rust/15 Python, quatro reexecuções de corpus e demais gates nativos aprovados. GUI extraída: 5,488 s, pico observado 75,12 MiB do processo, áudio desligado e PNG 1024×768 limitado ao monitor único. Não qualifica áudio instalado, interação manual, processo filho ou sessão longa; ver WINDOWS.md |
| Diagnósticos e GUI em VM Windows | Experimental; smoke sem áudio aprovado em 26/10/2026 | Release original `windows-v0.1.0-alpha.2`/fonte `bcb3f45`, hashes conferidos. Windows 10 LTSC 19044 x64/VirtualBox 7.2.16: CLI/IPC e GUI com motor separado, PNG 1280×800 em DX12/CPU, zero frames e encerramento normal. Máximos amostrados de working set: GUI 75,70 MiB, motor 10,90 MiB. Smoke anterior à instalação do cabo; release original ainda enumera listas vazias após instalá-lo. Investigação e correção local posteriores não herdam esse passe; ver `VALIDATION.md` |
| Instalação VB-CABLE e diagnóstico de identidade na VM | Instalação e enumeração corrigida observadas | Snapshot frio sem RAM criado, restauração não exercitada; Pack45 x64 instalado/reiniciado, driver assinado 3.3.1.7. Probe isolou envelope `{2}.`; a correção conservadora posteriormente executada no candidato enumera dispositivos físicos e dois alvos VB-CABLE. Os seis defaults pós-instalação foram consultados antes da passagem USB; sem promoção de compatibilidade universal |
| Candidato com correção de identidade WASAPI | Experimental; CI e diagnósticos na VM aprovados | `cbe42d5`, código `cca3270`: [Windows](https://github.com/aaronmlf/voice-app/actions/runs/40335940556) passou 138 Rust/15 Python e demais gates; [Linux](https://github.com/aaronmlf/voice-app/actions/runs/40336998729) passou 150 Rust/51 Python e integração PipeWire isolada. ZIP/PE conferidos no host e convidado; diagnósticos sem áudio passaram, inclusive após passagem USB. Release original e seus testes permanecem separados |
| HDA emulado desta VM a 48 kHz | Bloqueado para o perfil nativo; percurso USB testado separadamente | Windows mostrou apenas 16/22,05/44,1 kHz no mix. Os [caps declarados pela Oracle](https://github.com/VirtualBox/virtualbox/blob/b6bea72111fcff48f31c57c9f66365b1d3301bb9/src/VBox/Devices/Audio/DevHdaCodec.cpp#L239) omitem 48 kHz. USB direto permitiu conectar microfone/fones explícitos no perfil de 48 kHz; voz física permaneceu mutada, sem aceitação de seus efeitos |
| Saída Windows por VB-CABLE externo | Experimental; recepção sintética local observada | No candidato `cbe42d5`, Audacity 3.7.9 isolado recebeu tons e fala SAPI previamente processada offline via soundboard, em WAV estéreo/48 kHz/16 bits; loopback registrou pré-escuta nos fones, sem qualificar isolamento absoluto. Ensaios de 41,139/61,461 s tiveram underruns/overruns, descritos em `VALIDATION.md`. Não houve escuta subjetiva, aceitação de voz física, Discord, latência ou estabilidade prolongada |
| Fala sintética offline no Windows | Processamento e transporte numérico observados | Voz instalada do Windows gerou frase própria; Clean/Robot originais produziram WAVs mono/48 kHz/F32, depois importados com hashes conferidos e reproduzidos no soundboard. Correlação numérica no cabo/fones não comprova qualidade percebida, inteligibilidade ou DSP do microfone ao vivo |
| Reabertura e controles manuais no Windows | Parcialmente observados | Mesma biblioteca/IPC reabertos em 4,361 s com cinco clips/frames/hashes e seleções USB/cabo preservados, captura inicialmente desligada. Conexão e disparo observados na GUI; pré-escuta/Stop manuais inconclusivos por expiração do prazo. Encerramento normal de GUI/motor/keeper em 1,407 s. Discord bloqueado por login; ASR indisponível sem reconhecedor instalado. VM desligada e USBs devolvidos; revogação das ACLs temporárias pendente de autenticação administrativa |
| Candidato de estabilidade WASAPI | Experimental; gates aprovados, estabilidade operacional pendente | `30d26b8` separa playback de gravação/replay no Windows. Regressão de captura bloqueada, gates locais e CIs Windows (154 Rust/15 Python) e Linux (166 Rust/51 Python e PipeWire isolado) aprovados; pacote conferido no host. A rodada anterior `9954ec9` falhou por +1 `soundboard_underruns` após prontidão. Novos 60 s/30 min não iniciados por indisponibilidade USB; critérios e contadores preservados. |
| Repintura da GUI Windows | Política implementada; redução de CPU não qualificada | Áudio ativo 200 ms, ociosa 400 ms e edição/trabalho pendente 100 ms; eventos nativos continuam despertando a interface. Linux mantém a política anterior. As amostras curtas não demonstram redução de CPU nem estabilidade prolongada |
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
Naquela publicação, cabo instalado, Discord/Audacity/Tenacity, hardware,
Ouvir-me/pré-escuta, recursos ativos e latência permaneciam pendentes. Os ensaios
posteriores de 26/10 são do candidato `cbe42d5`, separados desta release.
O pico nativo de 75,12 MiB é apenas da GUI curta com áudio desligado. A meta de
80 ms continua aberta; não há promoção a beta nem retomada do driver próprio.


Atualização de integração DSP de 27/10/2026: vocoder, granular e classificação local de
coleções estão sendo integrados à base pública com as correções de estabilidade WASAPI.
Os gates locais anteriores dessa frente DSP passaram 179 testes Rust e 52 Python,
formatação, Clippy, limites, benchmarks e regressão PipeWire privada. Esses números
não são os testes da integração nem substituem os resultados Windows existentes.
Até registrar novos gates, o candidato integrado permanece sem aprovação de CI/release.
Apenas as 12 vozes builtin e suas artes acompanham a fonte; receitas, imagens e áudios
pessoais ficam fora do GitHub. Nenhuma qualificação de quatro horas, beta, fala física,
Discord/Audacity/Tenacity com os novos efeitos ou meta física de 80 ms foi acrescentada.

## Integração pública Linux — 27/10/2026

Fonte pública integrada às correções Windows atuais: formatação, Clippy com
avisos negados, limites entre componentes, **198 testes Rust e 52 Python**
aprovados em Linux. Sete perfis de benchmark DSP e a regressão PipeWire privada
passaram, incluindo vocoder/ambos os modos granular no consumidor virtual,
soundboard após DSP e pré-escuta privada. Pacote Linux desta fonte pública
gerado e inspecionado por checksum, binários e documentação; nenhum arquivo
de biblioteca pessoal ou preset customizado foi incluído. Operações locais
seriais sob limite de 3 GiB/sem swap, sem hardware ou GUI pessoal.

A CI integrada Ubuntu/Windows MSVC, o smoke nativo estendido dos três efeitos e
a inspeção dos candidatos remotos ainda são gates próprios. Estes resultados
locais não qualificam fala percebida, Windows 10/11 físico, VB-CABLE instalado,
Discord/Audacity, latência física ou sessões longas, nem promovem a versão a beta.

## Validação da publicação — 27/10/2026

A [PR #6](https://github.com/aaronmlf/voice-app/pull/6) foi integrada à `main`;
o código compartilhado também está em `beta` e `windows-native`. Na fonte
`1308b0d622eefb2ef28a8202b7db97cb0e9cf127`, a
[CI Linux](https://github.com/aaronmlf/voice-app/actions/runs/40526368003) passou
com 198 testes Rust, 52 Python, formatação, Clippy, boundaries, corpus, sete
benchmarks, política de dependências/licenças, PipeWire privado e pacote. A
[CI Windows](https://github.com/aaronmlf/voice-app/actions/runs/40526366912) passou
com 186 testes Rust no check nativo, corpus, benchmarks, licenças, checksum/PE,
smoke do pacote extraído (vocoder, dois modos granular, migração 11/12 → 13,
CLI/IPC/MCP e persistência) e GUI com renderização por software.

A primeira tentativa Wine expirou baixando dependências Ubuntu, antes de executar
o aplicativo. Só esse job foi repetido, preservando o candidato nativo. Na segunda
tentativa, os comandos offline passaram e a recusa exata do ambiente sem DACL
protegida foi verificada. **GUI/IPC e compatibilidade Wine não foram qualificadas.**
Hardware Windows 10/11, VB-CABLE instalado, Discord/Audacity/Tenacity, fala percebida,
latência física e sessões longas continuam gates próprios; nenhuma promoção a beta.

Após essa CI, mudou apenas este registro documental. As receitas distribuídas
continuam sendo as 12 builtin; biblioteca, presets, imagens e áudios pessoais
não foram publicados.
