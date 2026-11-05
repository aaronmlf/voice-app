# Publicar uma release

O repositório público é [aaronmlf/voice-app](https://github.com/aaronmlf/voice-app).
O código próprio usa MIT OR Apache-2.0. Apenas os 12 presets originais do aplicativo são
distribuídos; vozes criadas pelo usuário, gravações, modelos e bibliotecas pessoais ficam locais.

## Preparar a origem

1. Revise o código público, o `Cargo.lock`, a versão base em `workspace.package.version`
   e as notas das plataformas que serão publicadas.
2. Integre a mudança revisada à `main` pelo [fluxo de contribuição](../CONTRIBUTING.md).
   Confira os checks Linux e Windows relevantes e anote o SHA revisado.
3. Escolha uma tag inédita, com a mesma versão base do Cargo. Verifique tags e releases
   existentes, incluindo rascunhos. Use uma tag nova a cada publicação.

## Publicar por plataforma

Em **Actions**, abra o workflow da plataforma e selecione **Run workflow → main**.
Preencha a tag explicitamente. Para publicar Linux e Windows da mesma fonte, confirme
que os dois disparos selecionaram o mesmo SHA da `main`.

| Workflow | Formato da tag | Notas | Assets |
|---|---|---|---|
| [Linux experimental release](../.github/workflows/release.yml) | `vMAJOR.MINOR.PATCH`, com sufixo opcional | [docs/RELEASE-NOTES.md](RELEASE-NOTES.md) | Linux x86_64 `.tar.gz` e `SHA256SUMS.txt` |
| [Windows experimental release](../.github/workflows/release-windows.yml) | `windows-vMAJOR.MINOR.PATCH-alpha.N` | [packaging/windows/README.md](../packaging/windows/README.md) | Windows x64/MSVC `.zip` e `SHA256SUMS.txt` |

Os workflows são manuais e usam o SHA exato do disparo na `main` para validar,
compilar e publicar. Os dois executam checks de código, corpus, benchmarks e
verificações de licenças. O Linux também executa a política de dependências e
a integração PipeWire isolada; o Windows verifica o pacote extraído, a GUI sem
áudio e o comportamento offline/recusa segura no Wine.

Depois dos gates, cada workflow cria um rascunho, anexa o pacote e seu checksum e
publica como pré-release. Confira a tag, o SHA, os nomes dos assets e os checksums.
Atualize os links de download do [README](../README.md) para a plataforma publicada.
Se a `main` avançar antes da publicação, o workflow para antes de criar a tag;
revise a nova ponta da branch e inicie outro run.

O Linux é compilado no Ubuntu 24.04 x86_64 e depende de PipeWire e das bibliotecas
gráficas nativas. O Windows é compilado com MSVC em Windows Server 2022; o ZIP não
inclui VB-CABLE nem driver próprio. Os dois workflows usam um job de compilação
Cargo por runner e release sem LTO. O ambiente de compilação e os smokes de
software não qualificam áudio em hardware. A promoção a estável exige evidências
dos critérios do roteiro e alteração deliberada desse processo.

## Integridade e repetição

Somente o job de publicação recebe permissão de escrita. Ele cria a tag no SHA validado,
cria a release e envia os assets; os jobs de compilação usam leitura. O job de publicação
baixa apenas o artefato desse run, exige os
dois arquivos esperados e verifica o SHA256 antes do upload. Não usa token pessoal nem sobrescreve
assets existentes. Preserve as tags e os binários históricos. Se uma tentativa deixar uma tag
ou um rascunho, examine o run e o estado da publicação antes de repetir com outra tag inédita.

Os assets são anexados ao rascunho antes da publicação, conforme a recomendação do GitHub para
[releases imutáveis](https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository).
As permissões seguem a documentação de
[GITHUB_TOKEN](https://docs.github.com/en/actions/tutorials/authenticate-with-github_token).

## Evidências

As notas devem separar testes offline, grafo de software e testes físicos. Sessões curtas
continuam sendo smoke, mesmo quando aprovadas. Atraso de CPU, buffering algorítmico e latência
física são grandezas distintas. Discord, GUI OBS, hardware, outras distribuições e sessões
longas só entram como aprovados quando houver relatório específico.

Revise a lista explícita de arquivos antes de enviar o código público: fontes, fixtures próprias verificadas,
capturas da interface com vozes originais, documentação pública e licenças. Relatórios privados
podem conter caminhos, dispositivos e trabalho com presets pessoais; publique um resumo revisado.
Verifique também que a autoria do commit usa o endereço noreply do GitHub ou outro endereço
explicitamente público.
