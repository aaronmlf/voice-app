# Como contribuir

O voice-app é um aplicativo independente de voz, local e de código aberto. A versão Linux é
experimental. Consulte o [roteiro](docs/ROADMAP.md), a [matriz de funcionalidades](docs/FEATURES.md)
e o [contrato de desenvolvimento](AGENTS.md) antes de propor uma mudança.

Contribuições pequenas são mais fáceis de revisar. Para uma mudança de arquitetura, novo formato
de preset, dependência ou funcionalidade ampla, abra uma discussão em uma issue primeiro e descreva
o comportamento esperado. Informe se você está trabalhando na tarefa para evitar trabalho duplicado.

## Preparar o ambiente

Os requisitos de compilação e as dependências do Linux estão no [README](README.md).
Use o toolchain declarado pelo projeto e preserve o `Cargo.lock`. O aplicativo funciona offline;
baixar dependências de desenvolvimento pode exigir rede, mas não deve virar uma exigência para
o usuário executar o núcleo do produto.

Neste host de referência de 16 GiB, execute **uma única tarefa pesada por vez**: build, teste,
benchmark, empacotamento ou validação gráfica. Isso também vale para agentes e terminais distintos.
Não inicie outra tarefa enquanto houver uma validação prolongada em andamento.

Execute Cargo pelo wrapper:

```sh
bash scripts/cargo-low-memory.sh xtask check
```

O wrapper exige systemd de usuário, limita a árvore de processos a 3 GiB, desativa swap para a
tarefa e usa um job de compilação. Se o limite ou o lock falhar, investigue a causa; não contorne a
proteção. Reutilize `target/` no disco. Não coloque builds em `/tmp`, que pode ser tmpfs e consumir
RAM. O perfil release mantém LTO desativado para reduzir o pico de memória.

## Limites entre componentes

| Componente | Responsabilidade |
|---|---|
| `voice-core` | Contratos serializáveis, atômicos e filas limitadas; sem SO, GUI ou banco de dados |
| `voice-dsp` | DSP preparado; depende do core e das bibliotecas DSP, sem GUI ou adaptador de plataforma |
| `voice-pipewire` | Grafo de dispositivos Linux e callbacks; sem GUI, banco ou I/O de arquivos nos callbacks |
| `voice-library` | Decodificação, biblioteca persistente, intercâmbio e mistura offline |
| `voice-engine` | Registro de comandos, sessão, socket local, gravação e workers |
| `apps` | Clientes do motor; use o registro compartilhado, sem reimplementar comandos |

Combine contratos públicos antes de dividir tarefas entre colaboradores. A revisão de integração
inclui manifests, lockfile e a política de dependências. Novas dependências precisam de justificativa
e de verificação de licença.

## Regras de áudio em tempo real

Prepare memória e recursos fora dos callbacks. Um callback não pode alocar, liberar recursos
alocados no heap, bloquear, registrar logs, fazer I/O ou decodificar arquivos. Use filas com
capacidade fixa e atômicos; áudio não finito deve virar silêncio. Descreva como a mudança preserva
essas propriedades quando tocar no percurso de áudio.

Não capture a saída do microfone virtual como sua própria entrada. Não altere automaticamente o
microfone padrão do sistema. A monitoração começa desligada e a captura do desktop exige seleção
explícita. Use instâncias privadas do PipeWire nos testes automatizados, preservando os dispositivos,
padrões e configurações de áudio da pessoa que executa o teste.

## Validar uma mudança

Escolha verificações que exercitem o comportamento alterado. Para um candidato a release, execute
os gates abaixo **em sequência**, esperando cada comando terminar:

```sh
bash scripts/cargo-low-memory.sh xtask check
bash scripts/cargo-low-memory.sh xtask bench
bash scripts/cargo-low-memory.sh xtask package
```

`check` cobre formatação, Clippy, testes e limites de dependências. O benchmark mede tempo de CPU
do DSP; não mede latência física ponta a ponta. Quando alterar o grafo, a captura, os comandos ou
a persistência do percurso de áudio, rode também a regressão isolada, em uma etapa separada:

```sh
bash scripts/cargo-low-memory.sh xtask audio-check
```

Os [procedimentos de validação](docs/VALIDATION.md) explicam os validadores que usam o binário
pronto, os limites de memória, a medição por software e as sessões prolongadas. Uma execução curta
não aprova 60 minutos nem quatro horas. Não afirme compatibilidade com Discord/OBS, atalhos no
Wayland, latência física ou estabilidade prolongada sem a execução correspondente. Uma alteração
apenas documental pode ser conferida por leitura e links, sem repetir a bateria inteira.

Registre evidências e lacunas em [DEVLOG.md](docs/DEVLOG.md) e [FEATURES.md](docs/FEATURES.md).
Separe resultados offline, grafo privado, GUI e hardware. Relatórios brutos podem incluir caminhos
locais; revise-os antes de anexar a uma issue ou pull request. Não envie gravações pessoais,
conversas de terceiros, credenciais, banco da biblioteca ou dumps completos do ambiente. Prefira
uma reprodução com sinais sintéticos e os menores trechos de diagnóstico necessários.

## Enviar um pull request

Explique o problema, o comportamento após a mudança e como reproduzir ou verificar o resultado.
Liste os comandos realmente executados e suas limitações. Para alterações visuais, uma captura
com dados fictícios ajuda na revisão. Para alterações de áudio, informe formato, taxa, perfil de
efeitos e contexto de teste; use uma amostra sintética ou com licença identificada.

Mantenha o escopo pequeno e evite mudanças de formatação em arquivos alheios à tarefa. Se usar
agentes de IA, coordene os arquivos editados e as tarefas pesadas. A autoria da ferramenta não
substitui revisão, testes ou conferência de licenças e dados que serão publicados.

## Licenças e releases

Ao enviar código próprio, você concorda em disponibilizá-lo sob **MIT OR Apache-2.0**, conforme as
licenças do projeto. Código, fontes, imagens, sons e pesos de modelos podem ter termos distintos.
Inclua a origem e a licença de qualquer conteúdo novo; não envie mídia ou modelos sem autorização
e licença verificáveis. Referências a outros produtos não autorizam copiar sua identidade ou assets.

Uma release experimental deve trazer notas com capacidades testadas, limitações e requisitos,
o pacote Linux e seu SHA256. Os gates e o pacote precisam corresponder ao código da tag publicada.
Não promova uma versão a estável ou beta apenas porque compilou ou passou nos testes de software;
os critérios de hardware, GUI, distribuição e uso prolongado continuam separados no roteiro.

Relatos de vulnerabilidade seguem a [política de segurança](SECURITY.md).
