# options-rs — le moteur gamma exposure en Rust

Port du moteur Python vers Rust, pour les options sur le Nasdaq-100 (NQ, CME) via
Interactive Brokers. Le Python reste en place et fait autorité **jusqu'à ce que le
port soit complet et vérifié** ; c'est lui qui produit l'oracle décrit plus bas.

Les transactions d'initiés (SEC) auront leur propre dossier — rien ici ne les
concerne.

## Ce que la découpe garantit

```
options-rs/
  crates/
    gex-core/       le calcul pur : Black-76, greeks, expositions   ← AUCUNE E/S
    gex-store/      lecture parquet des relevés
    gex-cli/        le lecteur, binaire `gex`
    gex-ib/         la source : décisions de collecte + réseau TWS
    gex-collector/  le démon : binaire `gex-collector`
  fixtures/         l'oracle : un relevé IB réel et ses résultats Python
```

## Parité, mesurée

```sh
cargo run -p gex-cli -- NQ --replay fixtures/nq-2026-08-25.parquet
```

Sur le relevé IB réel, les deux moteurs rendent les mêmes chiffres — GEX, zero
gamma, les quatre murs, charm, vanna — et sur quatre jeux d'options, dont
`--gamma-source published` qui emprunte un tout autre chemin de calcul.

Le dépôt Python posait la règle « **tout ce qui produit un chiffre est testable
sans réseau** » comme une convention, qu'une distraction suffisait à enfreindre.
Ici elle est structurelle : `gex-core` ne déclare aucune dépendance capable
d'ouvrir un fichier ou un socket. `gex-ib` dépend de `gex-core` ; l'inverse est
impossible à compiler.

Sa seule dépendance est `libm`, pour `erf` que la bibliothèque standard n'a pas.
C'est du Rust pur, sans accès au système.

## L'oracle, et pourquoi il existe

`fixtures/` contient un **relevé IB réel** — NQ, 25 août 2026, 318 lignes, prix du
future 29 267,25, collecté en différé depuis TWS — et les nombres que le moteur
Python en tire.

C'est ce qui rend la comparaison des deux moteurs concluante. Porter du
comportement jamais confronté à de vraies données ferait courir le risque que Rust
et Python se trompent **identiquement**, et personne ne le verrait : un GEX faux
d'un facteur entier reste un nombre plausible. Le dépôt a déjà payé cette leçon
une fois, avec le multiplicateur du contrat.

| Fichier | Contenu |
|---|---|
| `nq-2026-08-25.parquet` | la chaîne brute, telle qu'écrite par `ib_collector.py` |
| `nq-2026-08-25.expected.json` | GEX, zero gamma, murs, charm, vanna, et cinq Black-76 exacts |
| `chaine-synthetique.json` | l'oracle du moteur : une chaîne construite depuis des paramètres connus, et tout ce que Python en tire |

Les deux ont des rôles distincts. Le relevé IB prouve que le moteur tient sur des
données réelles, avec leurs trous et leurs échéances qui ne s'accordent pas. La
chaîne synthétique — skew injecté à -0,15, open interest asymétrique, trois
échéances dont une mensuelle réglée le matin — est celle contre laquelle
`analyse.rs` se vérifie, parce qu'on y connaît la vérité terrain : le test du skew
doit retrouver exactement le -0,15 qu'on y a mis.

Les tests n'y comparent jamais une sortie figée sans la comprendre : les jeux
d'essai restent construits depuis des paramètres connus, et l'oracle sert à
vérifier qu'on retrouve la vérité terrain — pas à graver une régression.

## État

| Module | Python | Rust |
|---|---|---|
| loi normale (Φ, φ) | `scipy.stats.norm` | ✅ `loi_normale.rs` |
| Black-76, vol implicite | `black76.py` | ✅ `black76.rs` |
| multiplicateurs | `black76.py` | ✅ `contrat.rs` |
| expositions gamma / charm / vanna | `greeks.py` | ✅ `greeks.rs` |
| temps restant, fuseaux, conventions | `analysis.py` | ✅ `temps.rs` |
| format pivot | `chain.py` | ✅ `chaine.rs` |
| filtres, murs, profil, zero gamma | `analysis.py` | ✅ `analyse.rs` |
| lecture parquet | `snapshots.py` | ✅ `gex-store` |
| lecteur, rapport de séance | `main.py` | ✅ `gex-cli` |
| suivi `--watch`, historique | `main.py`, `history.py` | ✅ `gex-cli` |
| écriture parquet | `snapshots.py` | ✅ `gex-store::ecriture` |
| décisions de collecte | `ib_data.py` | ✅ `gex-ib::decisions` |
| couche réseau TWS | `ib_data.py` | ✅ `gex-ib::client` |
| assemblage, fusion socle/vif | `ib_data.py` | ✅ `gex-ib::assemblage` |
| démon | `ib_collector.py` | ✅ `gex-collector` |
| validation du modèle | `validate.py` | ✅ `gex-store::validation` |
| graphiques | `plots.py` | ❌ abandonnés, volontairement |

### Ce que le collecteur écrit

```
snapshots/NQ/
  courant.parquet      le relevé vivant, réécrit toutes les 15 s
  barres.parquet       les chandeliers d'une minute
  niveaux.parquet      la trace des niveaux, un point par minute
```

Les deux séries existent pour une raison : le calcul rend un zero gamma, on
l'affiche, on le jette. Or **c'est la dérive qui porte l'information** — un zero
gamma à 29 400 ne dit rien seul, le voir monter de 29 200 pendant que le prix s'en
approche, si.

Elles sont **bornées à trente jours glissants** et élaguées à chaque écriture. Une
série qui grossit sans fin est un piège différé, et les archives horodatées ont
justement été désactivées pour cette raison. `--retention 0` n'écrit rien.

Un point par minute et non par relevé : le collecteur tourne toutes les quinze
secondes, ce qui ferait 5 760 lignes par jour pour une information qui bouge à
peine d'un tour à l'autre. La minute est aussi la granularité des barres, donc les
deux séries partagent le même axe de temps.

L'écriture passe par un fichier temporaire puis un renommage. Sans cela, un
lecteur qui ouvre pendant l'écriture verrait un parquet incomplet — et un parquet
incomplet se lit comme une séance qui s'arrête, pas comme une erreur.

Conception : `docs/superpowers/specs/2026-08-26-barres-et-niveaux-design.md`.

### La coupure quotidienne

TWS et Gateway se redémarrent **de force une fois par jour** — c'est ainsi qu'IB
recharge les définitions de contrats — et l'authentification expire le dimanche à
1 h heure de New York. Le collecteur traite les deux comme des événements
normaux : il annonce la perte, attend, se rattache, et **reprend son socle sans le
rebalayer**. L'open interest qu'il porte ne bougera pas avant la publication du
soir ; le relire coûterait trois à cinq minutes pour les mêmes chiffres.

L'attente double à chaque tentative jusqu'à cinq minutes, puis se remet à zéro dès
qu'une session tient deux minutes : un délai fixe servirait mal l'un des deux cas —
trop long pour un redémarrage de quelques secondes, trop court pour une attente
humaine du dimanche.

Un défaut mesuré en coupant TWS pendant que le collecteur tournait : **une
souscription lancée sur une connexion morte ne rend pas d'erreur, elle bloque.** Le
processus restait vivant, muet, et n'écrivait plus rien — pire qu'un plantage,
puisqu'un processus figé passe pour un processus qui travaille. D'où la
vérification de la passerelle avant chaque cycle.

Ctrl+C lève un drapeau que la boucle lit au tour suivant, et la connexion se ferme
proprement. Un arrêt brutal laisserait des souscriptions ouvertes côté IB, qui
consomment le quota de cent lignes jusqu'à ce que TWS les recycle. Un second Ctrl+C
n'attend pas la fin du cycle.

**Le portage est terminé.** Les modules Python de la colonne du milieu n'existent
plus ; ce tableau garde leur nom parce qu'il dit d'où vient chaque crate.

### Ce que le portage a déjà rendu impossible à écrire

`gex-core` déclare `chrono` **sans la feature `clock`** : `Utc::now()` n'existe
pas dans cette crate. Un calcul qui dépendrait de l'heure courante ne compile
pas, donc tout résultat est reproductible par construction — l'instant de
valorisation est toujours passé en paramètre.

`EcheanceNy` et `InstantReleve` sont deux types distincts, l'un en heure de New
York, l'autre en UTC. Les confondre faisait passer le décalage de fuseau pour du
temps restant : c'est le défaut qui donnait quatre heures de vie à un contrat
déjà réglé. Le compilateur le refuse maintenant.

`Position` regroupe les six flottants d'un contrat évalué. En liste d'arguments,
intervertir `vol` et `t` compilait sans un mot et rendait un chiffre plausible.

**Le risque annoncé sur la couche IB est levé.** Il n'y a pas à réimplémenter le
protocole TWS : la crate `ibapi` (MIT, maintenue) le porte, et les six messages
dont le collecteur a besoin y sont tous — `connect`, `contract_details`,
`option_chain`, `market_data` avec `generic_ticks(["101","588"])`, et
`switch_market_data_type`. Ce dernier point était le plus incertain : sans les
ticks génériques, **pas d'open interest**, et donc pas de GEX.

Ce qui reste à écrire est le câblage, pas le protocole.

**Rien dans `gex-ib` ne passe d'ordre.** Le collecteur consulte : il énumère des
contrats et souscrit à des cotations. La crate expose bien un constructeur
d'ordres — il n'est appelé nulle part.

### Trois pièges que seul un TWS réel révèle

`cargo run --example sonde -p gex-ib -- "AAAA-MM-JJ HH:MM:SS"` confronte le client
à une passerelle vivante. Les trois défauts ci-dessous compilaient, passaient les
tests, et ne rendaient rien — ou pire, auraient rendu des chiffres faux.

**Le `conId` seul ne suffit pas.** Une souscription sur un contrat réduit à son
identifiant rend l'erreur 200, « aucune définition de titre trouvée ». Le contrat
complet voyage donc à côté de sa clé. Sans ça, la souscription part, aucune donnée
n'arrive, et un contrat muet ne se distingue pas d'un contrat sans open interest.

**L'heure d'échéance vit dans le champ de date.** `ibapi` consolide les trois dans
`last_trade_date_or_contract_month` — `"20260827 15:00:00 US/Central"` — et laisse
`last_trade_time` vide. Lire le mauvais champ ne casse rien de visible : on
retombe sur la clôture supposée, et une mensuelle réglée le matin se décale de
sept heures.

**Les grecs bid/ask servent des sentinelles.** `DelayedBidOption` porte une IV de
-1 et un gamma de -2 quand la cote manque. Les absorber écraserait le calcul du
modèle par un nombre négatif. Seuls les ticks 13 et 83 — le modèle — sont lus.

## Tests

```sh
cd options-rs
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Il n'y a pas d'intégration continue : ces deux commandes se lancent à la main,
avant de pousser.

Aucun test ne touche au réseau, comme dans le dépôt Python. La méthode de Brent
est écrite ici plutôt qu'empruntée : soixante lignes, contre une dépendance qui
aurait coûté plus cher à auditer.

## Conventions

Celles d'`AGENTS.md` à la racine s'appliquent : français pour le métier, anglais
pour les conventions externes, et **les commentaires disent ce qui se passerait
sans le code**, pas ce que fait la ligne suivante.
