# Matriz de funcionalidades

“Verificada” indica o contexto de teste descrito, não equivalência de qualidade com outro produto.
Nenhum marco inteiro foi promovido a estável.

| Capacidade | Estado | Evidência / pendência |
|---|---|---|
| Contratos, atômicos e filas limitadas | Verificada em testes | Ordem, concorrência, saturação, wrap de contador e validação |
| DSP tradicional | Verificada offline; ao vivo experimental | Sinais sintéticos, extremos, preservação entre blocos e ausência de alocação Rust |
| Fonte virtual + captura | Experimental | Cliente externo e módulo real `pulse_input_capture` da libobs 32.2.2 receberam sinal no grafo privado; Discord, GUI OBS e hardware adiados |
| Monitoração e pré-escuta separadas | Experimental | Ganho, sinal idêntico em L/R e isolamento medidos em sink estéreo no daemon privado |
| Detecção e reconexão automática | Experimental | Metadata de padrões, troca durante a sessão, retorno do daemon, retomada entre processos e cancelamento por Stop testados em fixture privada |
| Volume de Ouvir-me | Experimental | Controle de 0–200% independente; valor e persistência verificados por CLI/motor |
| Interface nativa | Experimental | Compilação e captura gráfica; fluxo real completo ainda precisa de validação |
| Filtro de vozes originais | Verificada em GUI isolada | Clique na biblioteca mostrou somente os 12 presets builtin; Minhas vozes mostrou apenas vozes de usuário, sem trocar a voz selecionada. Presets pessoais ficam fora da distribuição pública |
| Imagens de presets próprios | Experimental | PNG/JPEG/WebP, recorte, limites, falhas e persistência verificados; imagem gerenciada após remover fonte e reiniciar, renderização circular e comandos CLI/MCP testados |
| SQLite e presets JSON | Verificada em testes | Persistência, versão, limites e dados inválidos |
| WAV/MP3/Ogg Vorbis/FLAC | Verificada offline | Fixtures próprias; mono48k, resampling, limites e corte |
| Painéis, loop e sobreposição | Experimental | Mixer e comandos testados; uso prolongado real pendente |
| Gravação processada e replay30s | Experimental | Worker/PCM, finalização e preservação; criar voz A/B→gravar→cortar→soundboard→reiniciar→reutilizar sem WAV externo aprovado por comandos no grafo privado |
| CLI e MCP | Verificada em testes locais | Mesmo registro; sessão parada, biblioteca e mensagens JSON-RPC |
| Pacote Linux `.tar.gz` | Experimental | Checksum validado; binários extraídos executados, GUI em Xvfb e CLI integrado ao PipeWire privado |
| Pitch/formante, chorus, denoise | Verificada offline; ao vivo experimental | Backend real, testes espectrais/ruído/zero alloc Rust; pitch88ms, denoise20ms, ambos108ms de buffering algorítmico |
| Reordenação de efeitos | Experimental | 11 blocos no Laboratório; validação de permutação, atualização atômica, diferença no áudio offline/PipeWire, persistência e importação de presets antigos testadas |
| Mistura de duas vozes | Experimental | Ramos alinhados, transições e persistência; áudio externo por 30 s sem novos erros; atraso de onset p95 101 ms no grafo privado, 24 eventos, perfil sem denoise; latência física e sessão longa pendentes |
| Intensidade por efeito | Experimental | 11 intensidades em A e B; bypass, wet e interpolação, alinhamento dry/wet de pitch, presets antigos e ausência de alloc/free Rust em processamento verificados |
| Atalho no menu Linux/KDE | Verificado nesta sessão | Entrada desktop e ícone instalados; abertura do binário pronto por gio, identidade voice-app, limite de 3 GiB e segundo clique sem duplicação verificados; pinagem manual pelo menu do KDE |
| Captura do desktop | Experimental | Sink explícito, estéreo L/R, WAV/replay independentes, isolamento do virtual, finalização na perda do daemon e captura desligada após reiniciar verificados em PipeWire privado; hardware e sessões longas pendentes |
| Validação reprodutível de áudio | Verificada em grafo privado | Ferramentas de fluxo, latência, libobs e estabilidade; básico de 60 min aprovado com 3.600,018 s ativos e nenhum novo contador após aquecimento. Mistura de quatro horas ainda em andamento. PCM é descartado e o replay é limitado. Ver `VALIDATION.md` |
| Atalhos globais / bandeja | Planejada | Portal Wayland e fallback CLI |
| IA de conversão de voz | Planejada | Worker, modelos/licenças/hashes e hardware medido |
| Windows/macOS | Planejada | Drivers/plugins, assinatura e hardware próprios |
| Comunidade/mobile | Planejada | Serviços e pareamento ainda não implementados |
| AppImage/Flatpak | Planejada | Distribuição inicial em tar.gz |

