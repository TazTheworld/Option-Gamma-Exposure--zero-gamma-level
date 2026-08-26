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

Le seul Python restant est `sec_data.py`, la lecture des déclarations d'initiés
SEC, qui n'a jamais eu de rapport avec les options et attend son propre dossier :

```sh
venv\Scripts\activate                 # Windows
pip install -r requirements.txt       # pandas, requests, rien d'autre
python -m pytest tests -q
```

**Ne jamais régénérer `requirements.txt` avec `pip freeze`.** Le moteur d'options
n'a plus aucune dépendance Python.

## Langue

Français pour le métier, anglais pour les conventions externes. Le partage est
net et déjà appliqué partout :

| Français | Anglais |
|---|---|
| `analyser`, `murs`, `expositions`, `croisements_zero` | `build_chain`, `front_month` |
| `charger`, `ecrire`, `archiver`, `fusionner` | `black76`, `TickType`, `Contract` |
| `perimetre`, `selection_vif`, `temps_restant` | `StrikePrice`, `OpenInt`, `conId` |

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
| `gex-store` | relevés parquet, historique, validation du modèle |
| `gex-collector` | le démon : socle quotidien, vif entretenu |
| `gex-cli` | le lecteur, binaire `gex` |

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
place** toutes les quinze secondes ; l'horodater créerait près de mille cinq
cents fichiers par jour, et le lecteur ne saurait lequel est le dernier sans
lister le dossier. Les archives horodatées sont désactivées par défaut : elles
servent à la recherche, pas au suivi de séance.

## Tests

**Aucun test ne touche au réseau, ni à TWS.** La suite doit passer sur une machine
qui n'a ni passerelle ni connexion — un test qui aurait besoin d'un appel réseau
teste la mauvaise chose. Il n'y a pas d'intégration continue pour le rappeler :
`cargo test --workspace` et `cargo clippy` se lancent à la main, avant de pousser.

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

## Ne jamais faire en silence

**Annoncé à l'écran, jamais fait en silence.** S'applique à toute substitution,
tout mode dégradé, toute approximation — données différées, gamma recalculé faute
d'être publié, échéance dont l'heure de règlement n'a pas été servie.

Et échouer bruyamment vaut mieux que rendre du vide : une chaîne vide donnerait un
GEX de zéro, qui est un chiffre et non une erreur. Un processus figé est pire
encore — il passe pour un processus qui travaille.

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

Ils ne décrivent pas seulement ce qui est retenu, mais **ce qui a été écarté et
pourquoi** — c'est ce qui évite de refaire deux fois le même arbitrage. Ceux du
collecteur IB décrivent le moteur Python d'origine ; leur raisonnement tient
toujours, les noms de fichiers non.

## Ce qui reste à faire

- **Une vraie séance.** Le collecteur n'a jamais tourné plus de quelques minutes
  d'affilée. La reconnexion est vérifiée en coupant TWS, pas sur vingt-quatre
  heures.
- **`sec_data.py` dans son propre dossier**, avec ses tests.
- **Le contexte de séance** — OHLCV, iv30, le ratio GEX/volume — a disparu avec le
  CBOE. Le schéma d'historique garde ses colonnes vides pour que les fichiers
  déjà écrits restent lisibles ; IB pourrait les servir.
