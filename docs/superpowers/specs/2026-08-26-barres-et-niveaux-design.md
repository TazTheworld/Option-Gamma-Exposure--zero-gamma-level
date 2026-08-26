# Barres de prix et série des niveaux

Conception, 26 août 2026.

## Ce qu'on cherche

Un écran de séance : les chandeliers du NQ, et par-dessus les niveaux que le GEX
produit — zero gamma, murs, régime. Aujourd'hui le dépôt calcule tous ces niveaux
et n'en garde aucune trace utilisable pour un graphique.

Ce document couvre **la donnée**, pas l'interface. Ce sont deux problèmes
distincts, et les mélanger conduirait à façonner le stockage d'après un rendu qui
n'existe pas encore.

## Ce qui manque, exactement

**Aucune série de prix.** Le dépôt n'a jamais eu les chandeliers du sous-jacent.
`price_data.py` servait des séances quotidiennes d'actions par une API tierce ; il
ne couvrait pas les futures et a été supprimé au passage tout-Rust. Un écran de
séance demande de l'intraday, à la minute ou mieux.

**Aucune trace des niveaux dans le temps.** `analyser()` rend un zero gamma, on
l'affiche, on le jette. `history.csv` en garde une ligne par relevé enregistré,
mais il est fait pour la validation — une ligne par séance en pratique, avec
vingt-cinq colonnes dont dix-sept servent à autre chose. Le tracer minute par
minute demande une série dédiée.

Or **c'est la dérive qui porte l'information**. Un zero gamma à 29 400 ne dit rien
seul ; le voir monter de 29 200 à 29 400 pendant que le prix s'en approche, si.

## Les décisions, et pourquoi

### IB sert les barres, et il n'y a pas d'autre source à chercher

Trois appels existent, et la crate `ibapi` les porte tous les trois :

| Appel | Ce qu'il donne | Usage ici |
|---|---|---|
| `historical_data(…).fetch()` | un bloc de barres passées | l'amorçage au démarrage |
| `historical_data(…).stream()` | le même bloc, puis les mises à jour | le suivi de séance |
| `realtime_bars(…)` | des barres de cinq secondes | non retenu, voir plus bas |

**`stream()` plutôt que `realtime_bars`.** Les barres temps réel d'IB sont figées à
cinq secondes et ne s'agrègent pas côté serveur : construire des barres d'une
minute demanderait de les recoller nous-mêmes, avec la question des trous quand la
connexion coupe. `stream()` rend directement la granularité demandée et **renvoie
l'historique d'abord** — donc l'amorçage et le suivi sont le même appel, ce qui
supprime la couture entre les deux.

**Le fuseau des horodatages est un piège connu.** Ce dépôt s'est déjà fait prendre
trois fois par une heure qui se dérobe. IB horodate les barres dans le fuseau de la
place, ou en UTC selon le format demandé ; le stockage sera **en UTC, toujours**,
comme `InstantReleve`, et la conversion se fera à un seul endroit.

### Le format : parquet pour les barres, parquet pour les niveaux

Le même que les relevés, et pour la même raison : `gex-store` sait déjà l'écrire et
le relire, les types sont exacts, et un fichier de plusieurs milliers de lignes s'y
lit sans le parser ligne à ligne.

Deux fichiers, parce que ce sont deux cadences et deux durées de vie :

```
snapshots/NQ/
  courant.parquet      le relevé vivant, réécrit toutes les 15 s   (existe)
  barres.parquet       les chandeliers, une ligne par minute
  niveaux.parquet      la trace des niveaux, une ligne par minute
```

**Pourquoi pas une seule table.** Les barres viennent du future et existent même
quand aucune option n'a été collectée ; les niveaux viennent de la chaîne et
n'existent que quand un socle est là. Les fondre obligerait à porter des trous dans
les deux sens, et à réécrire toute la table à chaque nouvelle minute.

**Pourquoi pas du CSV.** `history.csv` en est un et le reste — il est lu par des
humains et par la validation. Ceux-ci sont lus par une machine à raison de
plusieurs centaines de lignes par séance, et le typage exact du parquet évite
d'avoir à décider, à la relecture, si une colonne vide veut dire zéro ou rien.

### La cadence : une ligne par minute, pas par relevé

Le collecteur réécrit le courant toutes les quinze secondes. Écrire un niveau à
chaque fois ferait 5 760 lignes par jour pour une information qui bouge à peine
d'un tour à l'autre — l'open interest ne change pas de la journée, et le zero gamma
dérive de quelques points par heure.

Une minute donne 1 440 lignes par jour, se trace sans décimer, et correspond à la
granularité des barres. **Les deux séries partagent donc le même axe de temps**, ce
qui évite d'aligner deux échelles à l'affichage.

Le point écrit est celui de la fin de minute, pas une moyenne : on veut l'état du
book à un instant, pas un lissage qui n'a jamais existé.

### Ce que la série des niveaux porte

Le strict nécessaire pour tracer, et rien de plus. Chaque colonne doit répondre à
« qu'est-ce que ça dessine ? » :

| Colonne | Ce que ça dessine |
|---|---|
| `instant` | l'axe des abscisses, en UTC |
| `spot` | le prix au moment du calcul, pour recouper avec les barres |
| `zero_gamma` | la ligne qui dérive |
| `gex` | l'histogramme du bas, et le signe du régime |
| `charm`, `vanna` | deux séries de plus, même panneau |
| `call_wall`, `put_wall` | les zones de résistance et de support |
| `call_wall_oi`, `put_wall_oi` | les mêmes en open interest brut |

Pas de profil complet, pas de GEX par strike : ce sont soixante et cent quarante
valeurs par minute, soit deux ordres de grandeur de plus, pour quelque chose que
l'écran lira depuis `courant.parquet` — le profil ne s'affiche qu'à l'instant
présent.

### La rétention : bornée, et par le collecteur

Une série qui grossit sans fin est un piège différé. Les archives horodatées
viennent d'être désactivées précisément parce qu'elles s'accumulaient sans que rien
ne les efface.

Les deux fichiers sont donc **bornés à N jours glissants** (défaut : 30), et le
collecteur élague à chaque nouvelle journée de compensation — au même moment où il
rebalaie son socle, donc sans mécanisme supplémentaire. Trente jours de barres à la
minute font environ 43 000 lignes, quelques mégaoctets.

### Le collecteur écrit, personne d'autre

Une seule plume par fichier. Le lecteur lit, l'interface lira. Deux processus qui
écriraient le même parquet produiraient un fichier tronqué sans que rien ne le
signale — et un fichier tronqué se lit comme une séance qui s'arrête.

L'écriture est **atomique** : fichier temporaire puis renommage. Sans cela, un
lecteur qui ouvre pendant l'écriture voit un parquet incomplet. Le relevé courant a
le même besoin et ne le fait pas encore ; ce sera corrigé au passage.

## Architecture

Rien de nouveau côté crates. Trois ajouts, chacun là où il appartient :

| Crate | Ajout |
|---|---|
| `gex-ib` | `barres()` : l'appel `historical_data`, et la traduction des horodatages |
| `gex-store` | `series` : lecture et écriture des deux tables, et l'élagage |
| `gex-collector` | le fil des barres, et l'écriture d'un point de niveaux par minute |

`gex-core` **ne bouge pas**. Une barre de prix n'est pas un calcul ; l'y faire
entrer romprait la règle qui veut que cette crate ne connaisse ni fichier ni
réseau.

### Les décisions pures, testables sans TWS

Trois, et ce sont les seules qui portent un raisonnement :

```
faut_il_ecrire_un_point(dernier, maintenant) -> bool
```
Vrai quand la minute a changé. Comparer des durées écoulées dériverait : à quinze
secondes d'intervalle, quatre tours font parfois cinquante-neuf secondes et le
point saute une minute sur quatre.

```
elaguer(instants, maintenant, jours) -> plage à garder
```
Ce qu'on garde, en jours glissants. Sorti de l'écriture pour être testable sans
fichier.

```
recoller(anciennes, nouvelles) -> barres
```
IB renvoie la dernière barre plusieurs fois pendant qu'elle se forme : la même
minute arrive incomplète, puis complète. Écraser par horodatage plutôt qu'ajouter,
sinon la série porte des doublons dont le dernier seul est juste.

## Ce que ça permet ensuite

L'interface, qui fait l'objet de son propre document. Elle n'aura qu'à lire trois
fichiers — `barres.parquet`, `niveaux.parquet`, `courant.parquet` — et ne touchera
ni à IB ni au calcul.

C'est la même séparation que celle qui existe déjà entre le collecteur et le
lecteur, et pour la même raison : ce qui affiche ne doit pas dépendre de ce qui
acquiert.

## Risques

**1. Le différé.** Les barres arrivent avec quinze minutes de retard, comme le
reste. Un écran de séance affichera donc un passé récent, pas le présent. Ce n'est
pas un défaut de conception mais une limite de l'abonnement, et elle doit être
**écrite à l'écran** plutôt que subie — la règle du dépôt vaut ici comme ailleurs.

**2. Le rythme des barres hors séance.** NQ se traite près de vingt-quatre heures
sur vingt-quatre, mais pas toutes. `TradingHours` décide si les barres couvrent la
séance régulière ou l'ensemble ; on prendra l'ensemble, puisque le gamma ne cesse
pas d'exister la nuit. Les trous du week-end restent des trous, et l'affichage doit
les traiter comme tels plutôt que de tirer un trait entre vendredi et lundi.

**3. La première minute après une reconnexion.** Le fil des barres meurt avec la
connexion. À la reprise, `stream()` renvoie l'historique avant de reprendre le
direct : le recollage par horodatage suffit à combler le trou, à condition que la
coupure ait duré moins que la profondeur demandée. Au-delà — la coupure du dimanche
— le trou est réel et sera visible. C'est préférable à une interpolation qui
inventerait des prix.
