# Collecteur Interactive Brokers pour les options sur NQ

Conception, 25 août 2026.

## Ce qu'on cherche

Un GEX sur les options du future Nasdaq-100 (NQ, CME) qui se rafraîchit pendant
la séance, au lieu du relevé figé que produisent les sources actuelles.

Aujourd'hui, les options sur futures arrivent par deux chemins et aucun ne
convient à un suivi de séance : `cme_data.py` lit un export de règlement
téléchargé à la main, `databento_data.py` interroge une API facturée au volume —
donc chaque essai coûte de l'argent, ce qui décourage précisément le genre de
boucle qu'on veut construire. Les deux ne donnent que le règlement de la veille.

Interactive Brokers apporte trois choses qu'aucune des deux n'a : le temps réel,
un tarif forfaitaire, et un gamma **publié** — ni le CME ni Databento ne le
diffusent, `black76_gamma()` le recalcule faute de mieux. Pour la première fois
`--gamma-source` pourra confronter deux estimateurs sur du future, comme il le
fait déjà sur les actions.

## Les décisions, et pourquoi

**IB porte l'acquisition entière, socle compris.** Le fichier End-of-Day gratuit
du CME contient pourtant tout ce que le socle demande — open interest, volume,
settlement, volatilités implicites — et l'aurait fourni instantanément, sur toute
la chaîne, sans risque. Le choix retenu est de tout prendre chez IB : une seule
source, un seul point de défaillance, aucun fichier à télécharger. Le prix de ce
choix est un balayage de deux à trois minutes au réveil et le risque n° 1
ci-dessous. Le repli est décrit avec ce risque, et il ne coûte rien à préparer
puisque `cme_data.load_settlement()` existe déjà.

Ce que ce choix ne coûte pas, en revanche, c'est de la fraîcheur — et c'est ce
qui le rend tenable. L'open interest n'est pas une donnée de temps réel : la
chambre de compensation le calcule après la clôture et ne le publie qu'une fois
par jour. IB ne le fabrique pas, il relaie la publication du CME. Les deux
chemins servent donc le même chiffre, à la même heure, et le vif reste identique
dans les deux cas. Ce qui se joue entre eux, ce n'est pas le temps réel mais le
périmètre couvert, le délai de démarrage et le nombre de sources à tenir.

**Collecteur persistant, séparé du lecteur.** NQ se traite près de vingt-quatre
heures sur vingt-quatre, et IB Gateway se redémarre de force une fois par jour.
Un unique processus qui ferait acquisition et calcul perdrait son socle à chaque
redémarrage — soit un rebalayage de trois minutes par jour, et une interruption à
une heure qu'on ne choisit pas. Le collecteur encaisse la reconnexion et réécrit
son relevé ; le lecteur lit un fichier, et se moque de savoir si le collecteur
tourne.

**Le format d'échange existe déjà.** `snapshots.py` archive des chaînes brutes,
`main.py --replay` les rejoue, et le spot comme la date de valorisation voyagent
dans le fichier en colonnes constantes. Le collecteur n'invente donc aucun
format : il appelle `snapshots.sauver()`, et tout l'aval fonctionne sans une
ligne de changement.

**L'état vivant et l'archive sont deux fichiers différents.** `snapshots.chemin()`
horodate à la minute. Un collecteur qui réécrit toutes les quinze secondes y
créerait un fichier neuf à chaque minute, soit près de mille cinq cents par jour
et par sous-jacent — une archive illisible, et un lecteur incapable de savoir
lequel est le dernier sans lister le dossier à chaque fois. Le collecteur écrit
donc deux choses :

- **le courant**, à chemin fixe (`snapshots/NQ/courant.parquet`), réécrit à
  chaque rafraîchissement. C'est ce que le lecteur ouvre.
- **l'archive**, horodatée par `snapshots.chemin()` comme aujourd'hui, écrite
  toutes les quinze minutes. C'est ce qui alimente `--replay` et `validate.py`.

Cela demande une seule addition à `snapshots.py` : une fonction `courant(ticker)`
qui rend le chemin fixe. `sauver()` et `charger()` servent les deux cas sans
changement, le format étant identique.

**La sélection du vif se fait par le gamma, pas par la distance au spot.** Voir
« La mécanique » ci-dessous : le socle sait déjà où le gamma se trouve, s'en
priver pour un critère géométrique serait gaspiller quatre-vingt-dix lignes.

## Ce que le collecteur produit

Un relevé au format pivot du projet — les colonnes `COLUMNS` de `cboe_data`,
plus les grecs de `COLONNES_GRECS`. Rien d'autre : il ne calcule ni GEX, ni zero
gamma, ni murs, parce que `analysis.analyser()` fait déjà tout ça et que le
dupliquer créerait deux endroits où la même formule pourrait diverger.

Deux cadences, pour les deux fichiers décrits plus haut :

| | Intervalle par défaut | Réglage |
|---|---|---|
| Courant | 15 s | `--rafraichir` |
| Archive horodatée | 15 min | `--archiver` |

Quinze secondes parce qu'en dessous on réécrit un fichier de plusieurs centaines
de kilo-octets plus vite que personne ne le lit, et qu'au-delà le « temps réel »
qu'on paie à IB ne sert plus à rien. Les deux restent réglables : c'est le genre
de valeur qui se juge sur une séance, pas sur le papier.

## Architecture

### Deux modules

| Module | Rôle |
|---|---|
| `ib_data.py` | connexion, énumération des contrats, souscription par lots, et les quatre fonctions pures |
| `ib_collector.py` | la boucle : socle au réveil, vif entretenu, réécriture du relevé, reconnexion |

Deux fichiers plutôt qu'un, sur le précédent de `flow_tracker.py` : une boucle
d'échantillonnage y est déjà séparée de la source qu'elle échantillonne
(`cboe_data.py`). Un module qui ferait les deux passerait le millier de lignes et
mélangerait ce qui se teste hors ligne avec ce qui ne se teste qu'en ligne.

### Les quatre fonctions pures

Ce sont elles qui portent tout le raisonnement, et elles seules sont écrites
d'abord — sans réseau, sans Gateway, testées comme les 158 tests actuels. Le
patron est celui que `databento_data.py` documente explicitement : « fonction
pure, séparée de l'accès réseau pour être testable sans clé API ».

```
echeances_utiles(echeances, quote_date, dte_max, dte_min) -> [Timestamp]
```
Quelles échéances énumérer, donc combien d'appels à `reqContractDetails`. C'est
le seul usage fiable de `reqSecDefOptParams`.

```
perimetre(contrats, prix, plage) -> [contrats]
```
Quels contrats demander. C'est l'inversion que le projet n'avait jamais eu à
faire : le CBOE sert toute la chaîne et `filtre_echeances()` élague ensuite ; IB
oblige à élaguer **avant** de demander, sous peine de brûler le budget de lignes
sur des strikes sans intérêt. `--range` et `--dte-max` cessent donc d'être des
réglages d'affichage pour devenir le périmètre d'acquisition.

Cette fonction **filtre, elle ne fabrique pas** — et la distinction est le fond du
problème. Elle reçoit les contrats réellement cotés, ceux qu'un
`reqContractDetails` par échéance vient de rendre, et n'écarte que ce qui est hors
plage. Construire un produit cartésien à partir des strikes et des échéances de
`reqSecDefOptParams` compterait des dizaines de milliers de contrats jamais
cotés.

Une contrainte de marché s'y ajoute, qu'il ne faut pas prendre pour une anomalie.
Le CME ne liste, sur une échéance hebdomadaire NQ, que vingt-cinq strikes de part
et d'autre du règlement de la veille — au pas de vingt-cinq points, cela fait
environ ±2,5 %. Au-delà, les contrats n'existent pas : seules les échéances
mensuelles et trimestrielles portent les strikes lointains. Demander `--range
0.2` ne rend donc pas ±20 % sur les échéances courtes, et `perimetre()` doit le
savoir plutôt que de compter des contrats manquants comme une erreur.

```
build_chain(defs, ticks, futures_price, quote_date, rate) -> (df, prix, date)
```
Jumelle de `databento_data.build_chain()`, et pour les mêmes raisons : mêmes
entrées conceptuelles (des définitions de contrats d'un côté, des valeurs de
l'autre), même sortie au format `COLUMNS`, même Black-76 pour le gamma quand il
manque, même `_clean()` en sortie.

```
selection_vif(chaine, budget_lignes=90) -> [contrats]
```
Les contrats à garder souscrits en permanence, triés par `|gamma × OI|`
décroissant. Un critère géométrique — plus ou moins N strikes autour du spot —
serait plus simple mais dilapiderait des lignes sur des strikes sans open
interest, alors que le socle vient précisément de mesurer où le gamma se trouve.
La sélection est refaite quand le spot sort de la bande couverte, sinon les
lignes entretenues finissent par ne plus regarder là où ça se passe.

Le budget tombe mieux qu'on ne l'avait prévu. Quatre-vingt-dix lignes font
quarante-cinq strikes, soit environ ±2,5 % — exactement la grille que le CME
liste sur une échéance hebdomadaire. Le vif ne couvre donc pas un morceau autour
du spot : il couvre **toute la chaîne listée de l'échéance proche**. Le tri par
`|gamma × OI|` sert alors moins à choisir qu'à ordonner le recyclage quand le
spot glisse et que la grille se déplace.

Quatre-vingt-dix et non cent : le future lui-même consomme une ligne, le
recyclage en réclame quelques-unes le temps que les annulations soient prises en
compte, et saturer le quota fait échouer les souscriptions suivantes en silence
plutôt que bruyamment. Le budget est un paramètre, pas une constante — un compte
avec des Quote Boosters en a davantage, et le collecteur doit s'en servir sans
qu'on le recompile.

```
fusionner(socle, ticks_vif, spot) -> (df, spot)
```
L'open interest vient du socle, l'IV du vif là où on l'a et du socle ailleurs, le
spot du vif. Figer l'open interest en séance n'est pas une approximation : le CME
le calcule après compensation et ne le publie qu'une fois par jour, donc la
valeur du socle est la seule qui existe. Ce qui bouge vraiment en séance, c'est
le prix du future et l'IV — et l'IV loin de la monnaie bouge peu tout en pesant
peu, le gamma s'y effondrant.

`fusionner()` rend `(df, spot)`, soit exactement ce que `analysis.analyser()`
prend en entrée. **Aucune ligne ne change dans `analysis.py`, `greeks.py`,
`plots.py`, `history.py` ni `validate.py`.**

### La mécanique

**Socle**, au réveil du collecteur puis une fois par jour, après la publication
de l'open interest par le CME — préliminaire à 18 h heure de Chicago, officiel le
lendemain matin. Le socle est donc rebalayé au premier réveil qui suit 18 h CT,
et une seule fois par journée de compensation :

1. Résoudre le future NQ de première échéance : `conId` et prix.
2. Lister les échéances : `reqSecDefOptParams(underlyingSecType="FUT",
   underlyingConId=…)`, puis `echeances_utiles()` pour ne garder que l'horizon.
   C'est la seule chose pour laquelle cet appel est fiable — son union
   d'échéances est exacte, son union de strikes ne l'est pas.
3. Énumérer les contrats réellement cotés : **un `reqContractDetails` par
   échéance**, `secType="FOP"`, le strike laissé indéfini. Un contrat
   incomplètement défini fait rendre à IB tous ceux qui lui correspondent, avec
   leurs `conId`. Une vingtaine de requêtes, et la mise en garde d'IB sur le
   throttling ne s'applique pas : elle vise la requête ambiguë qui demande tous
   les strikes ET toutes les échéances d'un coup.
4. `perimetre()` élague les strikes hors plage. Il filtre ce qui est coté ; il ne
   fabrique aucun contrat.
5. Balayer par lots d'environ quatre-vingt-dix :
   `reqMktData(genericTickList="101,588", snapshot=False)`, attendre la
   stabilisation, lire, `cancelMktData`, lot suivant. Le streaming n'est pas un
   choix : le mode snapshot n'accepte aucun generic tick, donc aucun open
   interest. Et `"101,588"` plutôt que `"101"` seul parce que rien ne dit lequel
   des deux IB retient pour un FOP — les demander ensemble ne coûte aucune ligne.
6. `build_chain()`, puis `snapshots.sauver()`.

**Vif**, en continu :

1. `selection_vif()` sur le socle, `reqMktData` permanent sur le résultat.
2. À chaque tick, mettre à jour l'IV et le prix du future en mémoire.
3. Toutes les quinze secondes, `fusionner()` puis écriture du courant ; toutes
   les quinze minutes, écriture de l'archive horodatée.
4. Quand le spot sort de la bande couverte, refaire `selection_vif()` et
   recycler les lignes.

**Reconnexion** : à la coupure quotidienne de Gateway, le socle est en mémoire et
sur disque. Le collecteur se rattache et reprend le vif sans rebalayer.

### Côté lecteur

`python main.py NQ --replay snapshots/NQ/courant.parquet` fonctionne tel quel,
sans rien changer : le courant est un relevé au même format que les autres.

Deux ajouts pour que ce soit utilisable :

- **`--suivre NQ`**, raccourci qui résout le chemin du courant au lieu de le
  faire taper. Le chemin fixe existe précisément pour ça.
- **Lever l'exclusion entre `--watch` et `--replay`.** `main.py:343` refuse
  aujourd'hui de les combiner, avec une raison qui était juste : « une archive ne
  bouge plus ». Le courant, lui, bouge. L'exclusion doit donc porter sur les
  archives horodatées et non sur le courant, et le message d'erreur doit le dire.

C'est le seul endroit de tout le projet où le collecteur oblige à toucher du code
existant.

### Le mode différé

Sans abonnement CME temps réel, `reqMarketDataType(3)` avant toute souscription.
Les grecs différés arrivent sur le tick 83 et non 13 : les deux sont lus. Le mode
est écrit à l'écran à chaque relevé et inscrit dans le fichier — le projet a déjà
cette règle, énoncée dans `price_data.py` à propos des substitutions d'indice par
un ETF : annoncé à l'écran, jamais fait en silence.

## Contraintes IB, chiffrées

| Contrainte | Valeur |
|---|---|
| Lignes de données simultanées | 100 par défaut ; +100 par Quote Booster, 10 maximum |
| Cadence de requêtes | lignes ÷ 2 par seconde, soit 50/s par défaut |
| Grecs | tick 13 en temps réel, tick 83 en différé |
| Open interest | tick 27/28 via generic 101 (par contrat, vérifié) ; tick 86 via generic 588 pour les futures ; tick 22 déprécié |
| Taille du contrat NQ | ×20, déjà dans `cme_data.CONTRACT_SIZES` |
| Mode snapshot | **incompatible avec les generic ticks** — donc inutilisable pour l'open interest |
| Grille listée, échéances hebdo NQ | 25 strikes de part et d'autre du règlement, soit environ ±2,5 % |
| Redémarrage de Gateway | forcé une fois par jour |

Le budget de cent lignes est ce qui dimensionne tout : quatre-vingt-dix contrats
entretenus, soit environ quarante-cinq strikes, soit environ ±2,5 % autour du
spot sur une échéance, au pas de vingt-cinq points du NQ.

## Risques, du plus grave au moins grave

**1. L'open interest sur les options SUR FUTURES.** Le risque a été réduit par
vérification le 25 août 2026, et ce qu'il en reste est étroit.

*Ce qui est établi.* Le tick générique 101 rend bien l'open interest du contrat
souscrit, et non un agrégat du sous-jacent. La docstring de `reqMktData` dans
`ib_async` l'annonce — « 101 : `putOpenInterest`, `callOpenInterest` (for
options) » — et `thetagang`, un robot de vente d'options en production sur IBKR,
le confirme par l'usage : il branche sur le `right` du contrat pour filtrer
strike par strike sur un open interest minimum. Ce branchement n'aurait aucun
sens sur une valeur agrégée, et le filtre ne filtrerait rien. Le mapping
lui-même est lisible dans `ib_async/wrapper.py` : `SIZE_TICK_MAP` associe 27 à
`callOpenInterest`, 28 à `putOpenInterest`, 86 à `futuresOpenInterest`, et il est
indexé par `reqId`, donc par contrat souscrit.

*Ce qui reste ouvert.* Aucune documentation ne dit si une option sur future est
traitée comme une option — tick 101 — ou comme un instrument à terme — tick 588.
Aucun exemple public demandant le tick 101 sur un FOP n'a été trouvé ; les projets
qui souscrivent à des FOP demandent 100, le volume, jamais 101.

*La parade, gratuite.* Demander `genericTickList="101,588"` et lire les trois
champs. Les ticks génériques ne consomment aucune ligne supplémentaire : c'est la
même souscription. Quel que soit le tick qu'IB retient pour les FOP, on l'a.

*Le test qui tranche, sans écrire une ligne.* L'API n'est qu'un canal de
livraison : ce que TWS n'affiche pas, le socket ne le dépêche pas. Il suffit donc
d'ouvrir la chaîne d'options NQ dans TWS et d'ajouter la colonne Open Interest.
Si elle se remplit, l'API peut la servir. Sinon, aucune API ne le fera.

*Repli si le test échoue* : le socle prend son open interest du fichier
End-of-Day gratuit du CME, que `cme_data.load_settlement()` lit déjà, et IB ne
garde que le vif. Le reste de l'architecture est inchangé — c'est la raison pour
laquelle `fusionner()` sépare l'origine de l'OI de celle de l'IV.

**2. Énumérer les contrats FOP.** Réduit par vérification, et la solution a
corrigé une erreur de conception plus grave que le risque lui-même.

`reqSecDefOptParams` rend l'union des strikes et l'union des échéances du
sous-jacent, **jamais les couples existants**. Croiser les deux fabrique un
produit cartésien qui est un majorant, pas une chaîne : sur NQ à ±20 %, vingt-quatre
mille contrats dont la vaste majorité n'a jamais été cotée, puisque le CME ne
liste que vingt-cinq strikes autour du règlement sur les hebdomadaires. Souscrire
à ces fantômes coûterait des minutes pour récolter une erreur 200 par contrat.

La doc IB donne la sortie : un contrat incomplètement défini — le strike laissé
vide — fait rendre à `reqContractDetails` tous les contrats qui lui
correspondent, `conId` compris. Une requête par échéance suffit donc, soit une
vingtaine, et l'on obtient les contrats réels. La mise en garde d'IB contre
`reqContractDetails` vise la requête ambiguë qui demande tous les strikes ET
toutes les échéances ensemble ; une échéance à la fois est précisément la
granularité qu'elle recommande.

`reqSecDefOptParams` reste utile pour ce qu'il fait bien : lister les échéances.
Si `reqContractDetails` devait échouer sur les FOP, la liste des contrats se
déduit du fichier End-of-Day du CME.

**3. Le balayage du socle est long, et pour une raison qu'on ne peut pas
contourner.** Le mode snapshot de `reqMktData` n'accepte aucun generic tick — or
l'open interest en est un. Il est donc impossible de l'obtenir par une requête
ponctuelle : chaque lot doit être souscrit en streaming, attendu, puis annulé. Ce
qui coûte n'est pas le réseau mais l'attente de stabilisation, puisqu'il faut
avoir reçu l'open interest, l'IV et les grecs avant d'annuler — et qu'un contrat
illiquide ne répond parfois jamais, donc chaque lot paie un délai de garde.

Le calcul, avec la grille réellement listée plutôt qu'un produit cartésien : une
vingtaine d'échéances quotidiennes ou hebdomadaires à cinquante strikes et deux
côtés font environ deux mille cent contrats, auxquels s'ajoutent une ou deux
échéances mensuelles à grille plus large. Soit deux mille cinq cents à trois
mille cinq cents contrats, une trentaine ou une quarantaine de lots, et une à
deux minutes et demie. Le coût n'est pas « des milliers de contrats à cadencer »
mais « trente-cinq lots dont chacun attend ».

Le socle est écrit sur disque dès qu'il est assemblé : le perdre avant coûterait
tout le balayage.

## Ce que la mesure dit, et ce qu'elle ne dit pas

Le GEX calculé ici porte sur les seules options du future NQ. Ce n'est pas une
limite subie mais la méthodologie du domaine : FlashAlpha et MenthorQ calculent
leurs niveaux gamma NQ à partir des seules options sur futures, valorisées en
Black-76, en mettant le gamma dollar à l'échelle du multiplicateur ×20 du CME
plutôt que du ×100 des options actions. Aucune des deux ne mélange les sources ;
convertir un niveau QQQ ou NDX vers le NQ est chez elles un outil séparé. C'est
mot pour mot ce que `black76_gamma()` et `CONTRACT_SIZES["NQ"]` font déjà.

Reste à savoir ce que ce périmètre laisse dehors. L'open interest d'options est
massivement plus gros sur QQQ et NDX que sur les options NQ — non parce que le
future serait un marché secondaire, il mène le prix et cote presque en continu,
mais parce que QQQ vit dans OPRA et qu'un compte-titres ordinaire y accède, là où
les options sur futures demandent un compte futures. C'est une barrière d'accès,
pas une hiérarchie entre marchés.

Or la couverture, elle, est fongible. Un dealer qui doit acheter du delta contre
des calls QQQ vendus achète du NQ plutôt que des parts de QQQ : plus liquide,
moins cher en capital, ouvert la nuit, sans emprunt de titres — un contrat NQ
valant environ huit cents parts de QQQ. Le gamma naît donc sur QQQ et NDX, mais
une part de sa couverture atterrit sur le carnet du NQ. Ce qu'on mesure ici en
porte l'écho sans en porter le stock.

La seule précaution qui en découle est de lecture, et elle doit figurer au
README : le chiffre produit est le gamma des options NQ, pas tout le gamma qui
pèse sur le Nasdaq.

## Hors périmètre

- Passer des ordres. Le collecteur lit, il ne trade pas.
- Les autres produits CME (ES, 6E, CL, GC) : l'architecture les accepte sans
  changement, `CONTRACT_SIZES` les connaît déjà, mais ils ne sont ni visés ni
  testés ici.
- QQQ et NDX. La couche « quel contrat demander » est la seule qui changerait ;
  `build_chain()`, `selection_vif()` et `fusionner()` sont identiques.
- Remplacer `price_data.py` par l'historique IB. Chantier distinct, plus petit,
  à traiter une fois la connexion établie.

## Vérification

Les quatre fonctions pures se testent hors ligne, sur des trames fabriquées comme
`_frames_databento()` le fait déjà dans `tests/test_sources.py`. C'est la règle du
dépôt et de sa CI : « aucun test ne touche au réseau, la suite doit passer telle
quelle ».

Ce qui ne se teste pas hors ligne — connexion, énumération, souscription — est
vérifié à la main contre Gateway, dans cet ordre : la colonne Open Interest d'une
chaîne NQ **dans TWS**, avant tout code ; puis un probe API sur un seul contrat
avec `"101,588"` ; puis l'énumération ; puis un socle complet ; puis le vif sur
une séance.

## Dépendances

`ib_async`, fork maintenu d'`ib_insync`, déclaré en extra `[ib]` dans
`pyproject.toml`. Le cœur du projet reste à cinq paquets : `python main.py TSLA`
ne doit rien devoir à IB.

## Portage Rust

Le projet est destiné à passer en Rust. Cela n'impose aucun choix de transport :
`rust-ibapi` réimplémente le protocole TWS, activement maintenu, avec un client
Tokio et un client bloquant. Ce que le portage contraint, c'est la **forme** —
fonctions pures d'un côté, couche réseau mince de l'autre — et c'est déjà la
forme que le dépôt impose.

Le découpage collecteur / lecteur est aussi la ligne de découpe du portage : le
collecteur se porte en premier, là où `rust-ibapi` sert, sans toucher au calcul ;
le fichier de relevé reste le contrat entre les deux, quelle que soit la langue
de chaque côté.
