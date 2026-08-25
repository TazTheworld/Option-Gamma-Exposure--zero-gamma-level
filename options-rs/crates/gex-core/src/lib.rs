//! Le calcul pur du gamma exposure : formules, greeks, expositions.
//!
//! **Cette crate ne sait rien lire ni écrire.** Elle ne déclare aucune dépendance
//! capable d'ouvrir un fichier ou un socket, et c'est le point de la découpe : le
//! dépôt Python posait la règle « tout ce qui produit un chiffre est testable sans
//! réseau » comme une convention, qu'une distraction suffisait à enfreindre. Ici
//! le compilateur la refuse.
//!
//! L'acquisition vit dans `gex-ib`, la persistance dans `gex-store`. Elles
//! dépendent de cette crate ; l'inverse est impossible par construction.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod analyse;
pub mod black76;
pub mod chaine;
pub mod contrat;
pub mod greeks;
pub mod loi_normale;
pub mod temps;

pub use black76::Sens;
pub use contrat::multiplicateur;
