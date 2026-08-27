# L'écran de séance

Conception, 26 août 2026.

## Ce qu'on cherche

Voir les niveaux sur le prix. Le lecteur rend aujourd'hui sept nombres et trois
avertissements ; c'est exact et illisible d'un coup d'œil. Un zero gamma à 29 391
ne dit rien tant qu'on ne voit pas où le prix se trouve par rapport à lui, ni
depuis quand il dérive.

## Ce qui existe déjà, et qu'il suffit de lire

Le collecteur écrit trois fichiers, et l'écran n'aura besoin de rien d'autre :

| | |
|---|---|
| `barres.parquet` | les chandeliers d'une minute, trente jours glissants |
| `niveaux.parquet` | zero gamma, murs, GEX, charm, vanna — un point par minute |
| `courant.parquet` | la chaîne complète à l'instant présent |

Les deux premiers partagent le même axe de temps, à la minute. C'était le point
de la conception des séries, et c'est ce qui permet de superposer sans aligner
quoi que ce soit.

## Les décisions, et pourquoi

### Un serveur local, pas un fichier ouvert dans le navigateur

Une page ouverte en `file://` ne peut pas lire un parquet sur disque : le
navigateur l'interdit, et il faudrait de toute façon un décodeur parquet en
JavaScript. Un petit serveur lit les fichiers, les rend en JSON, et sert la page.

C'est aussi ce qui permet le rafraîchissement : la page redemande les données
toutes les quelques secondes, sans que rien ne surveille le disque.

**Le serveur ne calcule rien.** Il lit ce que le collecteur a écrit et le
convertit. Refaire l'analyse à chaque requête HTTP dupliquerait `gex-core` dans un
second chemin — deux chemins qui finiraient par diverger, et l'écran montrerait
alors autre chose que le lecteur.

### Le graphique : une bibliothèque, pas du dessin à la main

Des chandeliers avec panoramique, zoom, curseur et échelles se comptent en
milliers de lignes. `lightweight-charts` — la bibliothèque de TradingView,
Apache 2.0 — les fait, pèse une quarantaine de kilo-octets, et donne exactement
le rendu et les interactions attendus.

Elle est **servie depuis le disque**, pas depuis un CDN : l'écran doit fonctionner
sans réseau, comme le reste du dépôt. C'est une dépendance de plus, assumée : la
seule alternative honnête serait de redessiner des chandeliers, et ce n'est pas le
sujet du projet.

### Ce que l'écran montre, et dans quel ordre

Trois zones, du plus important au moins.

**Le prix, au centre.** Les chandeliers, et par-dessus :

- le **zero gamma** en ligne, avec sa trace des dernières heures — c'est sa dérive
  qui porte l'information, pas sa valeur ;
- les **murs** en lignes horizontales, call au-dessus, put en dessous ;
- le **régime** en teinte de fond : au-dessus du zero gamma la couverture amortit,
  en dessous elle amplifie.

**Le profil par strike, à droite**, en barres horizontales alignées sur l'axe des
prix. C'est ce qui montre *à quelle hauteur* le gamma se concentre, et pourquoi un
mur est là où il est. Il vient de `courant.parquet` et ne concerne que l'instant
présent — un profil passé n'aurait aucun sens, le book ayant changé.

**Les totaux, en bas** : GEX, charm et vanna en séries temporelles, sur le même axe
que le prix.

### Ce que l'usage a ajouté

Écrit après avoir regardé l'écran pendant une séance. Chaque point vient d'une
lecture fausse qu'il rendait possible.

**Chaque série dans sa bande, jamais sur un axe partagé.** GEX, charm et vanna
n'ont pas la même unité — dollars par mouvement de 1 %, dollars de delta par jour,
dollars de delta par point de vol. Sur un axe commun elles se croisent, et un
croisement se lit comme un événement alors qu'il ne dépend que du facteur
d'échelle. Les séparer supprime le faux signal sans rien perdre : le signe et la
dérive sont tout ce qui compte pour chacune. Même raison pour l'IV et le skew,
qui sont des pourcentages et vivent dans un panneau à part.

**Le régime peint en fond.** `analyser()` rend déjà le GEX sur une grille de
niveaux de spot : ce que serait la couverture *si* le prix allait là. Le zero
gamma n'en montre qu'un point — la frontière. La courbe entière montre les zones,
et c'est ce que le fond peint. Contrôle de cohérence à l'écran : la bascule
vert/rouge doit tomber exactement sur la ligne du zero gamma.

**Les murs par open interest, à côté de ceux du gamma.** Ils étaient calculés et
stockés depuis le début, mais jamais affichés. Ils répondent à « où y a-t-il le
plus de contrats » quand les murs gamma répondent à « où la couverture est-elle la
plus sensible ». C'est leur accord qui rend un niveau crédible.

**La bande ±1σ implicite**, en lignes de prix et non en série : la durée restante
diminue de minute en minute, donc une bande tracée dans le passé montrerait un T
qui n'était pas celui du moment. Elle ne vaut que pour maintenant.

Elle est étiquetée « 2/3 · +114 » et non « +1σ ». Le sigma est du jargon, et un
écran qui demande un glossaire n'informe pas : l'étiquette dit ce que la borne
signifie — le marché price environ deux chances sur trois de rester dedans.

**Toutes les bascules du régime, pas seulement celle qu'on retient.**
`croisements` est une liste, et le gamma total change de signe plusieurs fois dès
que les ailes sont chargées. N'en montrer qu'une se lit comme une bascule unique —
au-dessus la couverture amortit, en dessous elle amplifie — ce qui est faux quand
il y en a trois. Les autres sont tracées en trait fin, dans un orange assourdi qui
les rattache au zero gamma sans les confondre avec lui.

Ce sont les croisements du profil **peint**, donc les endroits exacts où le fond
change de couleur : les traits nomment ce que le dégradé montre déjà. Une bascule
à ±10 % du spot n'est pas dans le champ du fond, et l'écran ne prétend pas la
connaître — le lecteur en ligne de commande, lui, va jusqu'à ±20 % et les nomme
toutes.

**Le max pain**, en série, contrairement aux deux précédents : il ne dépend pas du
temps restant mais de l'open interest, et c'est sa **dérive** qui porte le sens. Un
max pain immobile que le prix rejoint ne dit pas la même chose qu'un max pain qui
se déplace vers le prix — le second signifie que des positions s'ouvrent.

L'écran ne le présente pas comme une cible. Il est décrit dans la légende par ce
qu'il est — « où les options de l'échéance proche valent le moins au règlement » —
et non par ce qu'on lui prête.

**Le flux et la position, dans deux panneaux distincts.** Le GEX, le charm et le
vanna disent ce que la couverture oblige les teneurs de marché à acheter ou
vendre. Le delta, le vega et le thêta disent ce que leur position **est**. Le
thêta en particulier n'engendre aucun flux : il ne dit pas quoi faire, il dit ce
que ne rien faire coûte. Les mélanger aux trois premiers laisserait croire qu'il
agit — la séparation est la seule façon de le dire sans une note de bas de page.

Ils occupent la rangée du bas en **deux demi-panneaux côte à côte**, avec la
volatilité, plutôt qu'une rangée de plus : trois bandes supplémentaires ne
valaient pas cent dix pixels de graphique de prix, qui reste le sujet. Un
demi-panneau montre la même plage de temps, comprimée, et c'est la dérive qu'on y
lit — pas une date.

Ce partage a coûté deux défauts, tous deux dans la même zone et tous deux mesurés
plutôt que devinés :

- Un panneau de 800 px plafonne à 1 600 barres — la bibliothèque impose un demi-
  pixel par barre au minimum — et rabote toute plage plus large. Tant que chaque
  graphique écoutait les autres, ce rabot faisait autorité et ramenait le prix au
  début de sa série. **Le prix conduit seul** désormais.
- Un graphique dont toutes les séries sont vides accepte la largeur d'une plage et
  en ignore la position. Le squelette qui aligne les axes porte donc une valeur
  constante, invisible, au lieu de simples instants.

### Ce que l'écran ne fera pas

**Pas de rejeu.** Le curseur se promène sur les trente jours de séries, mais le
profil par strike reste celui de maintenant. Reconstituer un profil passé
demanderait d'archiver la chaîne entière chaque minute, ce que la conception des
séries a explicitement écarté.

**Pas de réglages.** L'horizon, la source de gamma et la convention de temps sont
des décisions qui changent le chiffre, et elles appartiennent à la ligne de
commande où elles sont explicites. Un menu déroulant les rendrait invisibles dans
une capture d'écran.

**Rien en silence.** Le différé de quinze minutes est écrit à l'écran, pas
supposé connu. Un relevé qui date, une série vide parce que le marché ne cote pas,
un collecteur arrêté : tout cela se dit.

## Architecture

Une crate de plus, et rien qui bouge ailleurs.

```
options-rs/crates/
  gex-web/
    src/main.rs      le serveur : trois routes, aucun calcul
    static/          la page, son style, et lightweight-charts
```

| Route | Rend |
|---|---|
| `/` | la page |
| `/api/barres` | les chandeliers |
| `/api/niveaux` | la série des niveaux |
| `/api/profil` | le GEX par strike de l'instant présent |

`gex-core` ne bouge pas. `gex-store` gagne au plus une fonction de lecture si le
profil par strike n'est pas déjà exposé.

## Risques

**1. Le différé rend l'écran trompeur.** Quinze minutes de retard sur un écran qui
ressemble à du temps réel est le piège le plus sérieux. L'horodatage du dernier
relevé est donc affiché **en permanence**, et vieillit visiblement quand le
collecteur s'arrête. Un écran qui a l'air vivant alors qu'il est figé est pire
qu'un écran vide.

**2. Le front n'a pas le filet des tests.** Le reste du dépôt est couvert ; une
page HTML ne l'est pas au même degré. La parade est de garder le JavaScript mince —
il traduit du JSON en séries et ne décide rien — et de mettre toute logique dans
le serveur, où elle se teste.

Vérifié à la livraison en faisant tourner la page dans un DOM contre le serveur
vivant : les sept valeurs remplies, 210 barres de profil, le strike le plus proche
du spot mis en évidence, aucune exception. Puis contre un dossier **sans relevé** :
rien d'inventé, l'avertissement affiché avec la commande à lancer. L'outil de
vérification n'entre pas dans le dépôt — il tirerait npm et jsdom dans un projet
Rust, pour un fichier que la conception garde volontairement mince.

**3. Une dépendance JavaScript vendorée.** Même question que pour `ibapi` : un
fichier de quarante kilo-octets figé dans le dépôt, contre une page qui ne
fonctionnerait pas hors ligne. Le choix va à l'autonomie, et le fichier est
identifié par sa version et sa licence.
