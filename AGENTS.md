# Conventions du dépôt

Gamma exposure et zero gamma level sur les options du Nasdaq-100 (NQ, CME), via
Interactive Brokers. Ce fichier consigne les règles que le code suit déjà — les
enfreindre casse la cohérence avant de casser les tests.

## Environnement

```sh
cd options-rs
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release                 # gex et gex-collector dans target/release/
```

**Il n'y a rien d'autre à installer.** Ni Python, ni environnement virtuel, ni
paquet : `cargo` suffit, et la chaîne complète — collecte, calcul, lecture — tient
dans les six crates.

Les déclarations d'initiés, qui vivaient ici, sont parties dans leur propre dépôt
(`insiders`) : elles ne partageaient aucun code avec le gamma exposure, et les
garder ensemble obligeait à lire deux README pour comprendre l'un ou l'autre.

## Langue

Français pour le métier, anglais pour les conventions externes. Le partage est
net et déjà appliqué partout :

| Français | Anglais |
|---|---|
| `analyser`, `murs`, `expositions`, `croisements_zero` | `build_chain`, `front_month` |
| `charger`, `ecrire`, `archiver`, `fusionner` | `black76`, `TickType`, `Contract` |
| `perimetre`, `selection_vif`, `temps_restant` | `StrikePrice`, `OpenInt`, `conId` |
| `attendu_implicite`, `pente_skew`, `iv_atm` | `max_pain`, `black76` |

Les commentaires et la documentation sont en français, **avec les accents**. Les
noms de colonnes du fichier parquet suivent le format CBOE et restent en anglais.

## Ce que les commentaires doivent dire

Le dépôt n'explique pas ce que fait le code — il explique **ce qui se passerait
sans lui**. C'est la règle la plus visible à la lecture, et la plus utile :

> « Le tri secondaire n'est pas cosmétique : à poids égaux, sans lui, deux appels
> rendraient deux listes différentes et le recyclage annulerait puis
> re-souscrirait les mêmes contrats pour rien. »

Un commentaire qui paraphrase la ligne suivante n'a pas sa place.

## Architecture

**Tout ce qui produit un chiffre est testable sans réseau.** C'était une
convention qu'une distraction suffisait à enfreindre ; c'est désormais le
compilateur qui l'impose.

| Crate | Rôle |
|---|---|
| `gex-core` | **aucune E/S possible** : Black-76, greeks, expositions, murs, profil, zero gamma |
| `gex-ib` | la source : décisions de collecte, et la couche réseau TWS |
| `gex-store` | relevés parquet, séries de barres et de niveaux, historique, validation |
| `gex-collector` | le démon : socle quotidien, vif entretenu |
| `gex-cli` | le lecteur, binaire `gex` |
| `gex-web` | l'écran de séance : sert les fichiers du collecteur en JSON, ne calcule rien |

`gex-core` ne déclare **aucune dépendance capable d'ouvrir un fichier ou un
socket**, et `chrono` y est déclaré sans sa feature `clock` : `Utc::now()` n'y
existe pas. Un calcul qui dépendrait de l'heure courante ne compile pas. Le
collecteur est le seul à lire l'horloge, et c'est ce qui rend tout le reste
reproductible.

Une formule ne s'écrit pas deux fois. Si une crate en a besoin, elle l'importe.

`options-rs/README.md` porte le détail de chaque crate, l'oracle du portage, et
les trois pièges que seul un TWS réel révèle.

### Ce que les types empêchent d'écrire

Trois erreurs qui compilaient en Python et ne compilent plus :

- **`EcheanceNy` et `InstantReleve`** sont distincts. Les confondre faisait passer
  le décalage de fuseau pour du temps restant — quatre heures de vie accordées à
  un contrat déjà réglé, sur des échéances portant 82 % du GEX.
- **`CleContrat` et `ContratOption`** sont distincts. Le vif se choisit sur une
  chaîne au format large, où le `conId` ne survit pas ; il faut passer par
  `avec_conid()` pour obtenir quelque chose de souscriptible. Côté Python, la
  boucle mourait au premier `reqMktData`.
- **`Position`** regroupe les six flottants d'un contrat évalué. En liste
  d'arguments, intervertir `vol` et `t` compilait sans un mot.

### Le patron de la source de données

Il n'y a qu'une source, mais toute source qu'on rajouterait suit cette forme, que
`gex-ib` documente en se découpant en trois modules :

- **`decisions`** — quoi demander, dans quel ordre, jusqu'où. Testé sans TWS.
- **`assemblage`** — des contrats et des valeurs vers une `Chaine`. Testé sans TWS.
- **`client`** — la couche réseau, qui ne fait que demander et traduire.

Ce qui décide doit être vérifiable sans passerelle, sans authentification et sans
marché ouvert.

### Le format pivot

Toute chaîne produite est une `gex_core::chaine::Chaine`. En aval, `analyser()` ne
sait pas d'où elle vient, et ne doit pas avoir à le savoir.

Les colonnes du fichier parquet gardent leurs noms hérités du CBOE —
`CallOpenInt`, `PutIV`, `StrikePrice`. Ils ne sont plus la vérité du calcul, mais
restent celle du fichier : les renommer rendrait illisibles des relevés que le
rejeu doit encore pouvoir ouvrir.

Le format est le joint entre le collecteur et le lecteur, et le seul endroit où
les deux se parlent. `snapshots/<PRODUIT>/courant.parquet` est réécrit **en
place** toutes les quinze secondes ; l'horodater créerait près de mille cinq cents
fichiers par jour, et le lecteur ne saurait lequel est le dernier sans lister le
dossier. Les archives horodatées sont désactivées par défaut : elles servent à la
recherche, pas au suivi de séance.

À côté vivent deux séries, `barres.parquet` et `niveaux.parquet`, un point par
minute chacune — la même granularité, donc le même axe de temps. Elles sont
**bornées à trente jours glissants** et élaguées à l'écriture : une série qui
grossit sans fin est un piège différé, et les archives viennent de le démontrer.

Toute écriture passe par un fichier temporaire puis un renommage. Un lecteur qui
ouvre pendant l'écriture verrait un parquet incomplet, et un parquet incomplet se
lit comme une séance qui s'arrête — pas comme une erreur.

## Tests

**Aucun test ne touche au réseau, ni à TWS.** La suite doit passer sur une machine
qui n'a ni passerelle ni connexion — un test qui aurait besoin d'un appel réseau
teste la mauvaise chose.

`cargo test --workspace` et `cargo clippy` **se lancent à la main, avant de
pousser**. L'intégration continue ne les remplace pas : elle ne fait qu'une chose,
rejouer les tests sur `aarch64` — l'architecture du Raspberry Pi —, et vérifier le
formatage. C'est la seule question que le poste, en x86_64, ne peut pas poser.

Les jeux d'essai sont **construits depuis des paramètres connus** — spot, IV, open
interest placés à des strikes choisis — pour vérifier qu'on retrouve la vérité
terrain. On ne fige jamais une sortie observée : un tel test passe encore quand le
calcul devient faux. Le test du skew, par exemple, doit retrouver exactement le
−0,15 qu'on a injecté.

Une formule ne se vérifie pas contre une constante tirée d'elle-même. Charm, vanna
et gamma sont recoupés par **différences finies** sur le delta et sur la prime. Les
deux formules du gamma — celle du call, celle du put — sont mathématiquement égales
et doivent coïncider : leur désaccord est le seul signal qu'une faute de frappe
dans l'une produirait.

`options-rs/fixtures/` garde un **relevé IB réel** et les nombres que le moteur
Python en tirait. C'est l'oracle du portage : sans lui, les deux moteurs auraient
pu se tromper identiquement sans que rien ne le révèle.

Ce qui ne se teste pas hors ligne vit dans `crates/gex-ib/examples/sonde.rs`, une
sonde manuelle contre un TWS vivant. Trois défauts n'ont été trouvés que par elle,
et un quatrième — le blocage sur connexion morte — en coupant TWS pendant que le
collecteur tournait.

**Le multiplicateur est le piège maison.** Se tromper de contrat ne produit aucune
erreur : un GEX cinq fois trop grand reste un nombre plausible. `gex_core::contrat`
refuse donc de deviner, et un produit inconnu est rejeté plutôt que doté d'un
défaut silencieux.

## Les mesures qui justifient les défauts

Les valeurs par défaut du lecteur ne sont pas des conventions : chacune a été mesurée,
et le chiffre est ce qui empêche de la changer par distraction.

**`--dte-max 30`.** Sur le SPX du 10 août 2026, la chaîne entière donnait +70,8 Md de
GEX quand le 0–7 DTE donnait −1,1 Md : deux régimes opposés pour la même séance. Les
échéances lointaines — strikes ronds à très gros open interest — dominent les murs sans
produire aucun flux de couverture à court terme.

**`--time-convention heures`.** La convention Perfiliev compte en jours ouvrés / 262
avec un plancher à un jour pour les 0DTE. Le gamma variant en 1/racine(T), ce plancher
fait une erreur massive. Mesuré sur le SPX, le rapport gamma recalculé / gamma publié
passe d'une médiane de 1,134 (77 % des contrats à plus de 10 % d'écart) à 1,000 (38 %).

**`--gamma-source iv`.** Seul choix cohérent de bout en bout : à un niveau de spot
hypothétique, aucun gamma publié n'existe, donc le profil ne peut être que recalculé.
Avec `published`, le total et le zero gamma viennent d'estimateurs différents — le
lecteur le dit alors, avec l'écart mesuré. Les deux se rejoignent à 1,7 % sur le 7 DTE
et divergent de 12,5 % sur toute la chaîne, là où `r = q = 0` cesse d'être neutre.

**Le répit de 1,2 s après l'annulation d'un lot.** `cancel()` envoie le message ;
TWS libère la ligne un peu plus tard. Sans ce répit, le lot suivant souscrit pendant
que le précédent compte encore, et le quota de cent lignes est atteint. Mesuré sur NQ :
quatre lots passaient, quatorze rendaient **1 260 contrats sans réponse** — un balayage
large échouait là où un balayage court réussissait, ce qui égarait le diagnostic.

**L'open interest se lit du côté du contrat, pas du dernier tick.** IB envoie les
**deux** ticks pour chaque contrat, celui qui ne le concerne pas à zéro, et le pertinent
en premier. Relevé sur NQ 29200 :

```text
contrat call : OptionCallOpenInterest = 53   puis OptionPutOpenInterest = 0
contrat put  : OptionCallOpenInterest = 0    puis OptionPutOpenInterest = 92
```

Retenir le dernier arrivé mettait **tous** les calls à zéro et laissait les puts justes
par hasard. Rien ne le signalait : une chaîne sans aucun call open interest se lit comme
un marché chargé en puts. Sur la même séance, le total passait de −813 à −351 M$, et le
zero gamma comme le call wall n'existaient tout simplement pas — sans calls, le gamma ne
change jamais de signe. Filtrer les zéros aurait « corrigé » le symptôme en inventant une
donnée sur les strikes réellement morts.

**Le seuil d'alerte à 5 %** sur l'écart entre sources, et **20 %** sur le poids des
0-1 DTE. En dessous, l'écart est du bruit ; au-dessus, il change la lecture.

**La bande des huit mesures est dessinée à la main, pas par la bibliothèque.** Trois
graphiques de largeurs différentes — 1600, 800, 800 — sous un prix de 1349 ne s'alignaient
sur rien : on ne pouvait pas descendre du regard d'un pic de prix vers ce qu'avait fait le
GEX au même instant, ce qui est pourtant la seule raison d'empiler des panneaux. Les
abscisses viennent maintenant de `gPrix.timeScale().timeToCoordinate()`, donc l'alignement
est exact **par construction**, à n'importe quel zoom. C'est déjà la méthode du fond de
régime et du profil par strike.

Trois pièges rencontrés en la construisant, tous mesurés :

- `timeToCoordinate` rend `null` pour un instant qu'aucune série ne porte. Les minutes où
  le sous-jacent ne traite pas n'ont pas de barre, donc pas de coordonnée : leurs points de
  niveaux **disparaissaient** de la bande. Une série vide portant les minutes des niveaux
  les rend à l'axe.
- La bibliothèque affiche l'heure **UTC** et ne sait rien faire d'autre. La bulle
  annonçait 20:05 sous un axe qui marquait 18:05. Les instants sont donc décalés à
  l'entrée du graphique et remis à la sortie, avec le décalage calculé **pour chaque
  instant** — entre juin et janvier il change d'une heure.
- **Une valeur extrême écrase l'échelle de toute la séance.** Mesuré : le cœur du charm,
  ses centiles 2 à 98, n'occupait que **13 %** de son étendue ; un unique pic de −17 Md$
  aplatissait la journée en un trait. L'échelle se borne donc au cœur — mais les points
  qui sortent sont marqués d'un chevron au bord du couloir et comptés dans la gouttière,
  à côté de l'étendue vraie. Rogner sans le dire effacerait le pic, ce qui serait pire que
  de l'aplatir.

**Zéro n'entre dans l'échelle d'un couloir que si les valeurs le traversent.** L'y forcer
quand elles restent d'un côté — le charm est négatif toute la journée — coûtait quatre-
vingt-dix pour cent du couloir pour une ligne dont on sait déjà où elle est. Quand zéro
n'y est pas, c'est le remplissage qui dit de quel côté : il s'ancre au bord du couloir
tourné **vers** zéro.

**Le max pain sur une seule échéance.** La valeur intrinsèque ne se cristallise qu'au
règlement, et deux échéances règlent deux jours différents : sommer leur douleur
supposerait que le prix est le même les deux jours — exactement l'hypothèse que le calcul
cherche à éclairer. C'est donc l'échéance la plus proche, comme pour l'IV ATM et le skew.

Deux propriétés que les tests fixent, parce qu'elles surprennent :

- **Le multiplicateur du contrat n'entre pas dans le calcul.** Il multiplie toutes les
  douleurs par le même facteur et ne peut pas déplacer le minimum. L'y faire figurer
  laisserait croire qu'il compte.
- **La douleur peut être plate.** Calls posés sur le strike le plus bas, puts sur le plus
  haut : tout est hors de la monnaie quel que soit le règlement envisagé, la somme vaut
  zéro partout et le max pain ne désigne rien. Le départage par distance au spot rend
  alors un strike stable — c'est une convention, pas une mesure, et le test le dit.

**`--valider` refuse de conclure sous vingt observations.** Deux garde-fous sans
lesquels la mesure se mesurerait elle-même : un périmètre à la fois — les enchaîner
classerait le régime d'après un GEX qui change de signe rien qu'en changeant
d'horizon — et une séance = une observation, parce que normalisé en racine du temps, un
mouvement réel de 0,2 % sur vingt minutes ressort à 1,7 % par jour.

## Ne jamais faire en silence

**Annoncé à l'écran, jamais fait en silence.** S'applique à toute substitution,
tout mode dégradé, toute approximation — données différées, gamma recalculé faute
d'être publié, échéance dont l'heure de règlement n'a pas été servie.

Et échouer bruyamment vaut mieux que rendre du vide : une chaîne vide donnerait un
GEX de zéro, qui est un chiffre et non une erreur. Un processus figé est pire
encore — il passe pour un processus qui travaille.

La règle vaut aussi pour ce qu'on fait aux fichiers déjà écrits. Quand le schéma
de `history.csv` change, `enregistrer` **migre l'en-tête** avant d'ajouter la
ligne — sans quoi les nouveaux relevés auraient un champ de plus que l'en-tête, et
`lire_historique`, qui cherche ses colonnes par nom, les aurait toutes décalées
d'un cran sans un mot. Les valeurs sont replacées **par nom**, une copie d'avant
est gardée, et le lecteur l'annonce à l'écran : un fichier qu'aucune source ne
redonnera ne se réécrit pas discrètement.

Le corollaire vaut pour les séries : quand le marché ne cote pas, la série des
niveaux **n'écrit rien** plutôt qu'un point à zéro. Une ligne plate se lirait comme
« le gamma est nul » là où la donnée dit « je ne cote pas ». Les barres, elles,
continuent : le future se traite la nuit.

Même règle pour les greeks de position. **Le delta, le vega et le thêta ne sont jamais
recalculés** — contrairement au gamma, qui a son repli en Black-76. Une source qui ne les
publie pas donne donc un total d'exactement zéro, qui se lit « le book est neutre » là où
la donnée dit « je n'en sais rien ». Exactement zéro sur des milliers de strikes ouverts
n'arrive pas par compensation : [`greeks_muets`] le détecte, le lecteur et l'écran le
disent. Dans la série, ces trois-là sont des `Option<f64>` et non des `f64` comme leurs
voisins, pour la même raison : un fichier écrit avant leur ajout n'en a aucune trace, et
les relire à zéro dessinerait une ligne plate sur toute la séance précédente.

**Le pas des chandeliers est un regroupement, pas une nouvelle collecte.** Le collecteur ne
stocke que la minute ; `agreger_barres` et `agreger_niveaux` en déduisent le reste.
Redemander à IB des barres de cinq minutes rendrait exactement la même chose contre une
requête de plus et un quota entamé.

Trois choses que les tests fixent :

- **Le seau se calcule sur le TEMPS, jamais sur le rang.** Grouper cinq barres consécutives
  paraît équivalent et ne l'est pas : il manque des minutes dès que le marché ne traite
  pas, et chaque trou décalerait tous les seaux suivants.
- **La frontière est celle de la séance, pas minuit UTC.** `debut_de_seance` la place à 17 h
  à New York, donc elle suit le changement d'heure — 21 h UTC l'été, 22 h l'hiver. Un
  chandelier journalier coupé à minuit UTC tomberait en plein après-midi américain : son
  ouverture ne serait pas l'ouverture, et il mélangerait la fin d'une séance et le début de
  la suivante. Pour les pas d'une heure ou moins la différence est nulle, la bascule tombant
  sur une heure ronde ; pour 4h et 1J elle décide de tout.
- **Les deux séries partagent leurs seaux**, par construction : c'est la même fonction. Si
  elles divergeaient, la bande du bas cesserait de s'aligner sur le prix — et l'alignement
  est toute sa raison d'être.

**Le volume différé porte un autre code que le volume direct.** `Volume` vaut 8,
`DelayedVolume` vaut 74, et le collecteur tourne en différé par défaut : le tick 8
n'arrive jamais. C'est exactement le piège déjà traité pour `ModelOption` (13) /
`DelayedModelOption` (83), et il s'est reproduit parce que rien ne le rappelait ailleurs.

Le symptôme est le pire possible : un volume nul **partout**, indistinguable d'un marché
qui n'aurait pas traité. Il n'a été trouvé que parce que TWS affichait à l'écran un volume
que le collecteur voyait à zéro. L'open interest, lui, n'a pas de variante différée — les
codes 27 et 28 arrivent tels quels, ce qui explique qu'il ait toujours fonctionné et que
rien n'ait mis la puce à l'oreille.

Le tick générique `100` porte les codes 29 et 30, le volume agrégé des calls et des puts.
Le volume du contrat lui-même n'en a pas besoin. Les trois codes demandés (`100`, `101`
open interest des options, `588` celui des futures) voyagent dans la même souscription et
ne consomment qu'une des cent lignes.

Et le volume tombe dans **exactement le même piège** que l'open interest, celui qui avait
mis tous les calls à zéro : IB envoie les deux codes pour chaque contrat, celui qui ne le
concerne pas à zéro. Le traitement côté-sensible est le même, et deux tests le fixent.

Aucune colonne n'a été ajoutée au format : `CallVol` et `PutVol` existaient depuis le CBOE
et s'écrivaient à zéro faute d'être collectées. Un relevé archivé avant leur remplissage les
rend nulles, ce que `colonne_ou_zeros` fait déjà pour une colonne absente.

**Un niveau hors échelle disparaît sans un mot : il lui faut donc un chiffre.** Les murs
sont tracés avec `autoscaleInfoProvider: () => null` pour ne pas tasser les bougies en
allant chercher un mur à mille points — mais du coup ils ne tirent pas le cadrage vers eux
et s'effacent purement et simplement dès qu'ils sortent de la fenêtre de prix. Un put wall
à 29 000 quand l'écran montre 29 400 à 29 650 n'existait alors **nulle part**. Chaque
famille de murs porte donc sa mesure dans l'en-tête ; c'est elle qui reste quand la ligne
sort du champ.

Et une ligne discrète n'est pas une ligne visible. Les murs par volume avaient hérité du
style volontairement effacé des murs par open interest — un pixel, pointillé sourd. Ce sont
pourtant les plus réactifs des trois. Deux pixels et une couleur franche.

**Le gamma majeur n'est pas un mur.** Un mur se calcule d'un seul côté — le gamma call
au-dessus du spot, le put en dessous — et sous contrainte de position par rapport au prix.
`gamma_majeur` prend le GEX **net** d'un strike, calls et puts confondus, sans regarder de
quel côté il tombe. Un strike peut donc être un mur call sans être le gamma long majeur, si
ses puts annulent ses calls. Un extremum du mauvais signe est écarté : sans aucun strike à
gamma net positif il n'y a pas de gamma long majeur, et rendre « le moins négatif » le
ferait passer pour un aimant.

**Le carburant s'intègre sur `ln S`, pas sur `S`.** Le GEX vaut des dollars de delta par
mouvement de 1 %, donc `d(delta$)/d(ln S) = 100 × GEX`. La grille des niveaux est linéaire
en prix : intégrer dessus rendrait un résultat faux d'un facteur `S`, avec la bonne forme et
les mauvais chiffres — l'erreur qui ne se voit pas à l'écran. Un test l'attrape en
confrontant le résultat à la définition même du GEX : à −100 M$ constants, monter de 1 %
doit forcer environ 100 M$ d'achats, pas 1 M$ ni 10 Md$.

Deux autres choses que les tests fixent : l'origine est le **spot** — sans ce recalage le
chiffre dépendrait du bord de la fenêtre d'analyse, qui n'est le lieu de rien — et le signe
est celui du **flux**, pas du delta. Les teneurs de marché traitent à l'inverse de leur
delta pour rester neutres ; positif veut donc dire qu'ils doivent acheter.

**Une couche nouvelle ne s'allume pas toute seule** quand elle en remplace une autre. Le
carburant prend la place du fond de régime : l'ajouter allumé aurait changé l'écran de
quelqu'un sans qu'il l'ait demandé, et il aurait cherché où son fond était passé. Les
couches déjà vues sont retenues dans le navigateur ; celles qui apparaissent ensuite avec
`defautMasque` arrivent éteintes — listées barrées, donc pas cachées pour autant.

**Le cadrage se compte en bougies, pas en durée.** Il se calait sur la durée couverte par
les niveaux, ce qui marchait tant que cette trace couvrait des heures : dès qu'elle repart
de zéro, vingt minutes de niveaux donnaient vingt bougies étalées sur douze cents pixels,
soit soixante pixels par chandelier — et le même calcul en journalier demandait de montrer
moins d'une bougie. Une durée ne veut rien dire tant qu'on ne sait pas quel pas on regarde ;
un nombre de bougies vaut pour la minute comme pour la séance. Entre 120 et 400, borné par
ce que la série contient.

Et l'espacement est **posé**, pas déduit d'une plage. `maxBarSpacing` existe dans les
options de `lightweight-charts` et n'y change rien — mesuré en 4.2.3, forcer un espacement
de 300 rend bien 300. Sans plafond, trois bougies journalières s'étiraient sur toute la
largeur à 427 px chacune : le graphique avait l'air cassé alors qu'il disait seulement
« je n'ai que trois jours ». `setVisibleRange` ne s'appliquant qu'à l'image suivante,
relire l'espacement pour le corriger après coup demanderait d'attendre une frame ; le
calculer d'avance n'attend rien.

L'horloge de l'en-tête garde la **dernière transaction non regroupée**. Le début du dernier
seau tomberait à 21 h la veille en journalier, et elle annoncerait une transaction vieille
de vingt heures.

**Une couche éteinte reste listée, barrée.** Le panneau des couches ne retire jamais une
entrée : une couche absente de la liste se lirait « il n'y a pas de put wall » au lieu de
« tu l'as éteinte ». Et son en-tête annonce le compte même replié — `couches 9/14 ·
5 masquées`, en couleur d'alerte — pour qu'une capture d'écran ne puisse pas mentir sur ce
qui manque. C'est ce qui rend le réglage acceptable là où la conception disait « pas de
réglages » : le problème était l'invisibilité, pas le réglage.

Le voile de la zone sans cotation, lui, **ne s'éteint pas avec le fond de régime**. Il ne
dit pas le régime, il dit que le sous-jacent ne traite pas : l'éteindre par préférence
d'affichage ferait disparaître un fait.

Et un couloir vide de l'écran **le dit** au lieu de rester muet — muet, on le lit comme une
mesure nulle. Avec deux formulations à ne pas confondre : « aucun point écrit » quand la
mesure n'existe nulle part dans la série, « hors de la fenêtre » quand le zoom l'a laissée
dehors.

**Le collecteur écrit un point de niveaux PAR HORIZON, pas un seul.** La chaîne n'est sous
la main qu'à l'instant du relevé : une fois le suivant écrit, celle-ci n'existe plus nulle
part. Calculer tout de suite ce que chaque horizon en dit coûte quelques millisecondes et
une quinzaine de méga-octets par mois ; le reconstituer après coup demanderait d'archiver
la chaîne entière — mesuré à **245 Ko le relevé, soit 353 Mo par jour**, contre 29 Ko pour
385 points de niveaux. `horizons_suivis` fixe la liste : les horizons courts, plus celui
du collecteur, jamais au-delà de ce qu'il a souscrit.

Sans cela, changer d'horizon à l'écran ne déplaçait que ce qui se recalcule depuis le
relevé courant. Le profil et le fond bougeaient, le zero gamma et les murs restaient où ils
étaient, et l'en-tête annonçait un mur à 29 500 pendant que le seul trait du graphique
restait à 29 800 : deux affirmations contradictoires à l'écran en même temps.

La colonne `dte_max` de la série est ce qui distingue les points d'une même minute. Sans
elle ils se liraient comme des mesures contradictoires du même instant. **Un point dont
l'horizon est inconnu — fichier écrit avant son ajout — n'est rangé sous aucun horizon** :
l'y mettre inventerait la donnée. Tant que le fichier n'en nomme aucun, ils passent tous ;
dès qu'un horizon apparaît, les muets sont écartés.

**Les boutons de l'écran viennent de la série, pas d'une liste écrite dans la page.** Les
deux divergeraient, et l'écran proposerait des horizons dont aucune trace n'existe. Il
reste des traits pleins étiquetés `call wall 1j` comme filet : ils ne servent que dans
l'intervalle où la série n'a pas encore de point à l'horizon demandé.

**Les archives ont leur propre fenêtre glissante, et c'est le point.** `--archiver` était un
piège différé : un relevé de 245 Ko à chaque cadence, et rien pour l'effacer. À la minute
c'est 353 Mo par jour qui s'ajoutent indéfiniment ; avec la fenêtre, 10,6 Go **stables**.

Elle est séparée de celle des séries — `--retention-archives` — parce que les deux coûts
n'ont rien de comparable : trente jours de séries pèsent une quinzaine de méga-octets,
trente jours d'archives à la minute en pèsent dix mille. Un chiffre unique obligeait à
sacrifier l'historique des niveaux pour borner celui des archives. Le défaut suit
`--retention` plutôt qu'une valeur à lui : un réglage qui s'écarterait en silence de celui
qu'on vient de poser serait une surprise.

Et le coût est **annoncé à la première archive écrite, mesuré sur elle** — pas estimé sur
une moyenne. Une chaîne NQ pèse dix fois une chaîne peu cotée, et personne ne devrait
découvrir le chiffre en regardant son disque se remplir.
`elaguer_archives` décide de l'âge par le **nom** — `2026-08-26_2145.parquet` — et non par
la date du fichier : une copie, une restauration ou une horloge remise à l'heure
changeraient la seconde, jamais le premier. Et seuls les noms de cette forme sont
candidats : `courant.parquet`, `barres.parquet` et `niveaux.parquet` vivent dans le même
dossier, et un balayage qui les prendrait pour des archives effacerait la séance en cours.
Un test le fixe.

**L'historique des expositions ne se télécharge pas.** L'API historique d'IB sert
`TRADES`, `MIDPOINT`, `BID`, `ASK`, `BID_ASK`, `AGGTRADES`, `HISTORICAL_VOLATILITY`,
`OPTION_IMPLIED_VOLATILITY`, `FEE_RATE`, `SCHEDULE` et `ADJUSTED_LAST` — **pas d'open
interest**. Or c'est lui qui fait l'exposition : sans lui, on a des greeks par contrat et
aucun GEX. L'open interest n'arrive que par les ticks 27 et 28, en direct. Reconstituer une
séance passée demanderait donc un fournisseur payant, pas un appel de plus. C'est la raison
pour laquelle la série se construit minute par minute et ne se rattrape pas.

**Un horizon vide n'est pas une panne.** Hors séance, « 0DTE » ne contient rien —
l'échéance du jour est déjà réglée. Rendre l'erreur d'`analyser` en 503 faisait afficher
« le serveur ne répond pas » sur tout l'écran pour un clic parfaitement légitime. La route
répond 200 avec un avertissement qui nomme l'horizon.

**Une barre naît d'une transaction, un point de niveaux s'écrit à l'horloge.** Elles ne
s'arrêtent donc pas ensemble, et l'écran porte deux horloges pour cette raison. Mesuré sur
une séance : 39 points de niveaux sans aucune barre, tous entre 21:06 et 21:44 UTC — l'arrêt
technique du CME, 17 h à New York —, avec deux valeurs de spot distinctes en 39 minutes.
Hors de ce bloc, aucun orphelin. Et le retard qui minimise l'écart entre le spot des
niveaux et la clôture des barres vaut **1 minute**, le temps qu'une barre se ferme : les
deux séries sont alignées, le différé n'y est pour rien. Sans ces deux dates côte à côte,
l'écran se lit « les options sont en avance sur le prix ».

[`greeks_muets`]: options-rs/crates/gex-core/src/analyse.rs

## Le partage entre les deux .md

`README.md` **présente** : ce que le projet mesure, comment le lancer, ce qu'il ne fait
pas. Il s'adresse à quelqu'un qui découvre.

`AGENTS.md` — celui-ci — porte **la technique** : architecture, conventions, tests,
pièges rencontrés, et les mesures qui justifient les défauts. Il s'adresse à qui va
modifier le code.

Un détail d'implémentation qui remonte dans le README est un signe qu'il manque une
section ici.

## Lecture seule

Le collecteur énumère des contrats et souscrit à des cotations. **Aucun ordre
n'est passé nulle part**, et `ibapi` expose pourtant un constructeur d'ordres : il
n'est appelé d'aucun endroit du dépôt.

## Git

Identité du dépôt :

```sh
git config user.name "Taz"
git config user.email "82410649+TazTheworld@users.noreply.github.com"
```

Messages de commit **en français sans accents** : un titre court à l'infinitif ou
au constat, puis un corps en prose qui explique **pourquoi** et ce que le
changement corrige. Pas de liste à puces, pas de résumé du diff — le diff est déjà
là. Terminer par le nombre de tests quand il change.

Le travail se fait directement sur `main`.

## Documents de conception

- Spécifications : `docs/superpowers/specs/AAAA-MM-JJ-<sujet>-design.md`
- Plans d'implémentation : `docs/superpowers/plans/AAAA-MM-JJ-<sujet>.md`
- Déploiement : `docs/raspberry-pi.md` — opérationnel et non conceptuel, il décrit
  une cible et non un arbitrage, d'où l'absence de date dans son nom.

Ils ne décrivent pas seulement ce qui est retenu, mais **ce qui a été écarté et
pourquoi** — c'est ce qui évite de refaire deux fois le même arbitrage. Ceux du
collecteur IB décrivent le moteur Python d'origine ; leur raisonnement tient
toujours, les noms de fichiers non.

## Ce qui reste à faire

- **Le sens réel du flux des teneurs de marché.** La convention en place — les
  dealers sont longs des calls et courts des puts — n'est pas une invention de ce
  dépôt : c'est celle de Barbon & Buraschi, *Gamma Fragility* (2020), qui mesurent
  l'effet et le trouvent maximal à **trente minutes**, l'horizon auquel les teneurs
  réajustent leur couverture. Elle reste pour autant **empirique et non mécanique** :
  les teneurs se couvrent aussi entre eux et avec d'autres options, et l'open
  interest dit où des positions ont été ouvertes, pas comment elles sont gérées
  aujourd'hui.
  Deux façons de la vérifier au lieu de la supposer. Classer les transactions contre
  le bid et l'ask rendrait le signe contrat par contrat, mais demande les données
  tick, que les jambes de spreads polluent. Le rapport *Traders in Financial Futures*
  de la CFTC donne, lui, la position nette de la catégorie **Dealer / Intermediary**
  sur `NASDAQ MINI`, futures et options combinés — hebdomadaire, public, gratuit.
  Agrégé, donc incapable de rendre un signe par strike ; mais suffisant pour
  confronter le signe global calculé ici à une mesure indépendante, semaine après
  semaine. Tant qu'aucune des deux n'est faite, le signe du GEX reste une hypothèse,
  et doit se dire comme telle.
- **Le contexte de séance** — OHLCV, iv30, le ratio GEX/volume — a disparu avec le
  CBOE. Le schéma d'historique garde ses colonnes vides pour que les fichiers déjà
  écrits restent lisibles ; IB pourrait les servir, et les barres en portent déjà
  une partie.
- **Le max pain n'est pas validé.** Il est mesuré, stocké dans la série des niveaux
  et affiché, mais `--valider` ne le confronte à rien : le « pinning » reste une
  théorie que ce dépôt décrit sans la juger. **Le verrou est levé** :
  `enregistrer` migre désormais l'en-tête d'un historique existant, en replaçant
  les valeurs par nom et en gardant une copie d'avant. Ajouter `max_pain` à
  `COLONNES` et au relevé suffit maintenant ; restent la quatrième affirmation à
  écrire, et l'échantillon à laisser se constituer.
