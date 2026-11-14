# Catálogo de vozes

`voices.json` contém as 18 receitas que acompanham os 12 presets embutidos em Rust.
O catálogo usa os mesmos IDs das vozes que já estavam disponíveis na coleção local.
Os seis presets de ficção e fantasia entram em **Vozes originais** quando são
adicionados a uma biblioteca. As 12 vozes temáticas entram em **Minhas vozes**.

`artwork/` guarda os desenhos vetoriais do projeto. A interface usa as miniaturas
PNG de 256 × 256 em `thumbnails/` e as carrega à medida que os cartões aparecem.
Denominações e personagens de Warhammer 40,000 são referências ao universo da
Games Workshop. As receitas são parâmetros de efeitos, com ilustrações feitas
para o Voice App.

Ao abrir uma biblioteca existente, o aplicativo adiciona somente IDs ausentes.
As receitas, categorias e imagens locais têm prioridade sobre o catálogo.
