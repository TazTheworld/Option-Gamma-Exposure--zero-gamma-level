# `lightweight-charts.js`

TradingView Lightweight Charts **v4.2.3**, build standalone en production.
Apache 2.0 — l'en-tête de licence est conservé en tête du fichier.

<https://github.com/tradingview/lightweight-charts>

## Pourquoi une copie

L'écran doit fonctionner sans réseau, comme le reste du dépôt. Un CDN ferait
dépendre l'affichage d'un tiers joignable, et une page qui ne s'affiche pas est
indiscernable d'un collecteur arrêté.

Le prix est 160 Ko figés. L'alternative honnête serait de redessiner des
chandeliers avec panoramique, zoom, curseur et échelles — quelques milliers de
lignes qui ne sont pas le sujet du projet.

## Ce dont la page dépend

`createChart`, `addCandlestickSeries`, `addLineSeries`, `setData`,
`timeScale().fitContent()` et `subscribeVisibleLogicalRangeChange`, plus
l'option `autoSize`.

La **v5 a supprimé `addCandlestickSeries` et `addLineSeries`** au profit d'un
`addSeries(CandlestickSeries, …)`. Monter de version demande donc de reprendre
`index.html` — ce n'est pas un remplacement de fichier.
