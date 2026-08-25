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
    gex-store/      parquet : relevé courant, archives, historique   (à venir)
    gex-ib/         la source : protocole TWS                        (à venir)
    gex-collector/  le démon : socle quotidien, vif entretenu        (à venir)
    gex-cli/        le lecteur                                       (à venir)
  fixtures/         l'oracle : un relevé IB réel et ses résultats Python
```

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
| filtres, murs, profil, zero gamma | `analysis.py` | ⬜ `analyse.rs` |
| parquet | `snapshots.py` | ⬜ `gex-store` |
| source IB | `ib_data.py` | ⬜ `gex-ib` |
| démon | `ib_collector.py` | ⬜ `gex-collector` |
| lecteur | `main.py` | ⬜ `gex-cli` |

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

**Le risque connu est la couche IB.** `ib_async` n'a pas d'équivalent Rust dont la
maturité soit établie ; l'alternative est d'implémenter le sous-ensemble du
protocole TWS dont le collecteur a besoin — connexion, `reqContractDetails`,
`reqSecDefOptParams`, `reqMktData` / `cancelMktData`, `reqMarketDataType`. Six
messages sur environ cent cinquante. C'est le seul poste dont le calendrier n'est
pas prévisible, et il est traité en dernier pour cette raison.

## Tests

```sh
cd options-rs
cargo test
cargo clippy --all-targets -- -D warnings
```

Aucun test ne touche au réseau, comme dans le dépôt Python. La méthode de Brent
est écrite ici plutôt qu'empruntée : soixante lignes, contre une dépendance qui
aurait coûté plus cher à auditer.

## Conventions

Celles d'`AGENTS.md` à la racine s'appliquent : français pour le métier, anglais
pour les conventions externes, et **les commentaires disent ce qui se passerait
sans le code**, pas ce que fait la ligne suivante.
