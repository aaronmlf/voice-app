# Publicar uma release

O repositório público é [aaronmlf/voice-app](https://github.com/aaronmlf/voice-app).
O código próprio usa MIT OR Apache-2.0. Apenas os 12 presets originais do aplicativo são
distribuídos; vozes criadas pelo usuário, gravações, modelos e bibliotecas pessoais ficam locais.

## Fluxo

1. Atualize `workspace.package.version` em `Cargo.toml`, as notas de versão e as evidências.
2. Envie o código revisado para `main`. Confira o workflow **Linux checks**.
3. Em **Actions → Linux experimental release → Run workflow**, escolha `main` e informe uma
   tag nova, por exemplo `v0.1.0-alpha.1`. Alternativamente, envie uma tag `v*` pelo Git.
4. O workflow fixa o commit de origem e confere a versão. Executa os gates de código,
   corpus, benchmark, dependências/licenças e integração PipeWire isolada nos runners GitHub.
5. Após todos os jobs passarem, cria um rascunho, anexa o `.tar.gz` e `SHA256SUMS.txt` e publica
   como pré-release. Uma falha anterior à publicação mantém o diagnóstico em Actions.

Os binários são compilados com Rust 1.98.1 no Ubuntu 24.04 x86_64, com um job Cargo e sem LTO.
Esse ambiente de compilação não certifica áudio em hardware Ubuntu. O aplicativo continua
dependendo de PipeWire e das bibliotecas gráficas nativas; o arquivo não é um AppImage.
Compilações locais podem exigir glibc mais nova e não devem substituir esses assets publicados.

O workflow aceita tags da mesma versão base do Cargo, incluindo sufixos de pré-release.
Enquanto o produto for experimental, ele sempre publica pré-releases. A promoção a estável
exige evidências dos critérios do roteiro e alteração deliberada desse processo.

## Integridade e repetição

O job de publicação recebe permissão de escrita somente para criar a release e enviar os
assets; os jobs de compilação usam leitura. Ele baixa apenas o artifact desse run, exige os
dois arquivos esperados e verifica o SHA256 antes do upload. Não usa token pessoal nem sobrescreve
assets existentes. Se uma tentativa deixar um rascunho, examine o run e o rascunho antes de
escolher uma nova tag ou concluir a recuperação manualmente.

Os assets são anexados ao rascunho antes da publicação, conforme a recomendação do GitHub para
[releases imutáveis](https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository).
As permissões seguem a documentação de
[GITHUB_TOKEN](https://docs.github.com/en/actions/tutorials/authenticate-with-github_token).

## Evidências

As notas devem separar testes offline, grafo de software e testes físicos. Sessões curtas
continuam sendo smoke, mesmo quando aprovadas. Atraso de CPU, buffering algorítmico e latência
física são grandezas distintas. Discord, GUI OBS, hardware, outras distribuições e sessões
longas só entram como aprovados quando houver relatório específico.

Revise a lista de arquivos antes do primeiro commit: fontes, fixtures próprias verificadas,
capturas da interface com vozes originais, documentação pública e licenças. Relatórios privados
podem conter caminhos, dispositivos e trabalho com presets pessoais; publique um resumo revisado.
