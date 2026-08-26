//! La source : Interactive Brokers, options sur futures CME.
//!
//! IB ne sert pas une chaîne — il sert des contrats un par un, avec un plafond de
//! cent lignes de données simultanées. Tout ce module découle de cette contrainte.
//!
//! Le fichier est coupé en deux, et la coupe suit la règle du dépôt : ce qui
//! produit un chiffre doit être testable hors ligne. [`decisions`] porte tout le
//! raisonnement et se vérifie sans TWS ; la couche réseau ne se vérifie qu'avec
//! une passerelle en marche, une authentification valide et un marché ouvert.
//!
//! Le protocole TWS lui-même n'est pas réimplémenté : la crate `ibapi` le porte,
//! et les six messages dont le collecteur a besoin y sont tous — connexion,
//! `contract_details`, `option_chain`, `market_data` avec ticks génériques, et
//! `switch_market_data_type`. L'écrire à la main aurait été soixante messages de
//! protocole binaire versionné pour le même résultat.
//!
//! **Rien ici ne passe d'ordre.** Le collecteur consulte : il énumère des
//! contrats et souscrit à des cotations, rien d'autre.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod client;
pub mod decisions;
