//! La couche réseau : ce qui parle à TWS.
//!
//! **Ne se teste pas hors ligne**, et c'est pour cela que tout ce qui décide vit
//! dans [`crate::decisions`]. Ici on ne fait que demander, attendre, et traduire
//! ce qui revient.
//!
//! Le protocole lui-même n'est pas réimplémenté : `ibapi` le porte. Ce module est
//! du câblage, et il tient dans ce que le sondage d'IB avait établi — un
//! `contract_details` par échéance, jamais un produit cartésien, et des
//! souscriptions en streaming par lots parce que le mode snapshot n'accepte aucun
//! tick générique, donc aucun open interest.
//!
//! **Rien ici ne passe d'ordre.** Le collecteur consulte.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use gex_core::black76::Sens;
use ibapi::Client;
use ibapi::contracts::tick_types::TickType;
use ibapi::contracts::{Contract, SecurityType};
use ibapi::market_data::MarketDataType;
use ibapi::market_data::realtime::TickTypes;

use crate::decisions::{
    CleContrat, ContratOption, echeances_utiles, instant_echeance, separer_echeance,
};
use gex_core::temps::InstantReleve;

/// Adresse par défaut de TWS. 7496 en live, 7497 en paper, 4001/4002 pour Gateway.
pub const ADRESSE_DEFAUT: &str = "127.0.0.1:7496";
/// Identifiant client par défaut, distinct de ceux qu'une interface humaine prend.
pub const CLIENT_ID_DEFAUT: i32 = 17;

/// Ticks génériques demandés à chaque souscription.
///
/// `101` porte l'open interest des options, `588` celui des futures. Rien dans la
/// documentation d'IB ne dit lequel il retient pour une option SUR future : on
/// demande les deux, ce qui ne coûte **aucune ligne supplémentaire** — c'est la
/// même souscription.
pub const TICKS_GENERIQUES: [&str; 2] = ["101", "588"];

/// Délai de garde par lot, en secondes.
///
/// Dix, et c'est mesuré : à quatre secondes vingt-quatre pour cent des contrats
/// restent muets, à six cinq pour cent, à dix moins d'un. Contre l'intuition
/// d'économiser du temps, la générosité gagne — parce qu'un contrat muet ne se
/// distingue pas d'un contrat sans open interest, les deux arrivant vides. Un
/// délai trop court ne produit donc pas une erreur mais **un GEX silencieusement
/// amputé d'un quart**.
pub const ATTENTE_LOT: Duration = Duration::from_secs(10);

/// Ce qui empêche de collecter.
#[derive(Debug)]
pub enum ErreurIb {
    /// La passerelle ne répond pas, ou a coupé.
    ///
    /// Ce n'est pas forcément une panne : TWS se redémarre de force une fois par
    /// jour, et le collecteur doit traiter la coupure comme un événement normal.
    Connexion(String),
    /// Une requête a échoué.
    Requete(String),
    /// Le produit demandé n'existe pas sur cette place.
    ProduitIntrouvable(String),
    /// Une échéance servie par IB est inexploitable.
    Echeance(crate::decisions::ErreurEcheance),
}

impl std::fmt::Display for ErreurIb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErreurIb::Connexion(m) => write!(
                f,
                "Connexion à TWS impossible : {m}. Vérifie que TWS ou Gateway \
                 tourne, que l'API est activée, et que le port est le bon."
            ),
            ErreurIb::Requete(m) => write!(f, "Requête refusée par IB : {m}"),
            ErreurIb::ProduitIntrouvable(p) => {
                write!(f, "Aucun future {p:?} coté sur cette place.")
            }
            ErreurIb::Echeance(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ErreurIb {}

impl From<crate::decisions::ErreurEcheance> for ErreurIb {
    fn from(e: crate::decisions::ErreurEcheance) -> Self {
        ErreurIb::Echeance(e)
    }
}

/// Une connexion établie à TWS.
pub struct Passerelle {
    client: Client,
    /// Vrai si les données servies sont différées.
    pub differe: bool,
}

impl Passerelle {
    /// Se connecte, et choisit le régime de données.
    ///
    /// Le différé est le défaut : il suffit au calcul et ne demande aucun
    /// abonnement. Le régime est annoncé à l'appelant plutôt que subi — le dépôt
    /// a une règle, un mode dégradé ne se prend jamais en silence.
    pub fn connecter(adresse: &str, client_id: i32, differe: bool) -> Result<Self, ErreurIb> {
        let client =
            Client::connect(adresse, client_id).map_err(|e| ErreurIb::Connexion(e.to_string()))?;
        if differe {
            client
                .switch_market_data_type(MarketDataType::Delayed)
                .map_err(|e| ErreurIb::Requete(e.to_string()))?;
        }
        Ok(Passerelle { client, differe })
    }

    /// La passerelle répond-elle encore ?
    ///
    /// À vérifier AVANT chaque cycle, et pas seulement en cas d'erreur. Une
    /// souscription lancée sur une connexion morte ne rend pas d'erreur : elle
    /// **bloque**, indéfiniment, et le collecteur reste vivant sans rien écrire
    /// ni rien dire. C'est pire qu'un plantage — un processus figé passe pour un
    /// processus qui travaille.
    pub fn vivante(&self) -> bool {
        self.client.is_connected()
    }

    /// Le future de première échéance : celui que les options suivent.
    pub fn front_month(&self, produit: &str, place: &str) -> Result<Contract, ErreurIb> {
        let gabarit = Contract {
            symbol: produit.into(),
            security_type: SecurityType::Future,
            exchange: place.into(),
            currency: "USD".into(),
            ..Default::default()
        };
        let mut details = self
            .client
            .contract_details(&gabarit)
            .map_err(|e| ErreurIb::Requete(e.to_string()))?;
        if details.is_empty() {
            return Err(ErreurIb::ProduitIntrouvable(produit.to_string()));
        }
        // La plus proche échéance, pas la première rendue : IB ne garantit pas
        // l'ordre, et se tromper de contrat ferait suivre un future qui ne porte
        // pas les options qu'on collecte.
        details.sort_by(|a, b| {
            a.contract
                .last_trade_date_or_contract_month
                .cmp(&b.contract.last_trade_date_or_contract_month)
        });
        Ok(details.remove(0).contract)
    }

    /// Les contrats d'options **réellement cotés** dans l'horizon demandé.
    ///
    /// Deux appels de nature différente, et l'ordre compte. `option_chain`
    /// d'abord, uniquement pour les échéances : il rend une entrée PAR CLASSE DE
    /// COTATION — quatorze sur NQ — et chaque classe ne porte qu'une échéance
    /// avec ses propres strikes. Ne lire que la première en fait manquer treize.
    ///
    /// Puis `contract_details`, une fois par échéance retenue, le strike laissé
    /// indéfini : IB rend alors tous les contrats de cette échéance avec leurs
    /// identifiants. C'est la seule façon d'obtenir les couples existants.
    /// Croiser les strikes et les échéances fabriquerait un produit cartésien —
    /// 11 872 contrats là où il n'en existe que 6 696, la majorité jamais cotée.
    pub fn enumerer(
        &self,
        futur: &Contract,
        place: &str,
        releve: InstantReleve,
        dte_max: Option<i64>,
        dte_min: i64,
        progres: bool,
    ) -> Result<Vec<(ContratOption, Contract)>, ErreurIb> {
        let chaines = self
            .client
            .option_chain(
                &futur.symbol.0,
                place,
                SecurityType::Future,
                futur.contract_id,
            )
            .map_err(|e| ErreurIb::Requete(e.to_string()))?;

        let mut toutes: Vec<NaiveDate> = Vec::new();
        let mut classes = 0usize;
        // Le flux porte aussi des avis non fatals - les codes 2100 a 2169 d'IB -
        // qui ne sont pas des donnees et ne doivent pas passer pour des echeances.
        for item in chaines {
            let Some(chaine) = item
                .map_err(|e| ErreurIb::Requete(e.to_string()))?
                .into_data()
            else {
                continue;
            };
            classes += 1;
            for e in &chaine.expirations {
                if let Ok(d) = NaiveDate::parse_from_str(e, "%Y%m%d") {
                    toutes.push(d);
                }
            }
        }
        let retenues = echeances_utiles(&toutes, releve, dte_max, dte_min);
        if progres {
            println!(
                "{classes} classes de cotation, {} échéances, {} dans l'horizon",
                {
                    let mut u = toutes.clone();
                    u.sort_unstable();
                    u.dedup();
                    u.len()
                },
                retenues.len()
            );
        }

        let mut contrats = Vec::new();
        for jour in retenues {
            let gabarit = Contract {
                symbol: futur.symbol.clone(),
                security_type: SecurityType::FuturesOption,
                last_trade_date_or_contract_month: jour.format("%Y%m%d").to_string(),
                exchange: place.into(),
                currency: "USD".into(),
                ..Default::default()
            };
            let details = self
                .client
                .contract_details(&gabarit)
                .map_err(|e| ErreurIb::Requete(e.to_string()))?;

            // L'heure est la même pour toute l'échéance : on la lit sur le premier
            // contrat qui la porte, et on la sert à tous. Elle vit dans le champ
            // de date — « 20260827 15:00:00 US/Central » — et non dans
            // last_trade_time, que TWS laisse vide sur les FOP.
            let servi = details
                .iter()
                .map(|d| separer_echeance(&d.contract.last_trade_date_or_contract_month))
                .find(|(_, heure, _)| heure.is_some());
            let (heure, fuseau) = match &servi {
                Some((_, h, z)) => (*h, *z),
                None => (None, None),
            };
            let echeance = instant_echeance(jour, heure, fuseau)?;
            let horaire = heure.is_some();

            let mut retenus = 0usize;
            for d in &details {
                let Some(sens) = sens_de(d.contract.right) else {
                    continue;
                };
                contrats.push((
                    ContratOption {
                        con_id: d.contract.contract_id,
                        cle: CleContrat {
                            echeance,
                            strike: d.contract.strike,
                            sens,
                        },
                    },
                    // Le contrat complet voyage avec sa clé : le conId seul fait
                    // rendre une erreur 200 à la souscription, et une souscription
                    // muette ne se distingue pas d'un contrat sans open interest.
                    d.contract.clone(),
                ));
                retenus += 1;
            }
            if progres {
                let defaut = if horaire {
                    ""
                } else {
                    "  (heure non servie : clôture supposée)"
                };
                println!(
                    "  {} : {retenus} contrats, échéance {} NY{defaut}",
                    jour.format("%Y%m%d"),
                    echeance.0.format("%H:%M")
                );
            }
        }
        Ok(contrats)
    }

    /// Souscrit à un lot, attend la stabilisation, lit, puis annule.
    ///
    /// Le streaming n'est pas un choix : le mode snapshot **n'accepte aucun tick
    /// générique**, donc aucun open interest. Chaque lot doit être souscrit,
    /// attendu, puis relâché — et ce qui coûte n'est pas le réseau mais l'attente,
    /// puisqu'un contrat illiquide ne répond parfois jamais.
    /// `lot` porte le contrat IB complet à côté de sa clé. Le `conId` seul ne
    /// suffit pas : IB rend une erreur 200, « aucune définition de titre trouvée ».
    /// C'est un piège discret — la souscription part, aucune donnée n'arrive, et
    /// un contrat muet ne se distingue pas d'un contrat sans open interest.
    pub fn collecter_lot(
        &self,
        lot: &[(ContratOption, Contract)],
        attente: Duration,
    ) -> Result<HashMap<i32, Valeurs>, ErreurIb> {
        let mut recu: HashMap<i32, Valeurs> = HashMap::new();
        let mut abonnements = Vec::new();

        for (c, contrat) in lot {
            let abonnement = self
                .client
                .market_data(contrat)
                .generic_ticks(&TICKS_GENERIQUES)
                .streaming()
                .subscribe()
                .map_err(|e| ErreurIb::Requete(e.to_string()))?;
            abonnements.push((c.con_id, abonnement));
        }

        let limite = Instant::now() + attente;
        while Instant::now() < limite {
            let mut recu_ce_tour = false;
            for (con_id, abonnement) in &abonnements {
                while let Some(item) = abonnement.try_next() {
                    recu_ce_tour = true;
                    // Un avis ou une erreur sur UN contrat n'arrete pas le lot :
                    // un contrat illiquide qui ne repond jamais est le cas normal,
                    // et abandonner le lot entier pour lui couterait les
                    // quatre-vingt-neuf autres.
                    let Ok(item) = item else { continue };
                    let Some(tick) = item.into_data() else { continue };
                    recu.entry(*con_id).or_default().absorber(&tick);
                }
            }
            if !recu_ce_tour {
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        // Annuler libère les lignes pour le lot suivant. Sans ça, le quota de
        // cent est atteint au deuxième lot et les souscriptions échouent — en
        // silence, ce qui est le pire des deux mondes.
        for (_, abonnement) in abonnements {
            abonnement.cancel();
        }
        Ok(recu)
    }
}

/// Ce qu'une souscription rend pour un contrat.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Valeurs {
    /// Volatilité implicite, en décimal.
    pub iv: Option<f64>,
    /// Gamma publié par IB — ce que ni le CME ni Databento ne servaient.
    pub gamma: Option<f64>,
    /// Delta publié.
    pub delta: Option<f64>,
    /// Vega publié.
    pub vega: Option<f64>,
    /// Theta publié.
    pub theta: Option<f64>,
    /// Open interest du contrat.
    pub open_interest: Option<f64>,
    /// Prix du sous-jacent, servi gratuitement dans chaque tick d'option.
    pub sous_jacent: Option<f64>,
}

impl Valeurs {
    /// Absorbe un tick.
    ///
    /// Les grecs arrivent par `OptionComputation`, l'open interest par les ticks
    /// de taille 27 (call), 28 (put) et 86 (future). Les trois sont lus parce que
    /// rien ne dit lequel IB retient pour une option SUR future — et se tromper
    /// ne produirait pas une erreur mais un open interest absent, donc un GEX nul
    /// sur les strikes concernés.
    fn absorber(&mut self, tick: &TickTypes) {
        match tick {
            TickTypes::OptionComputation(c) => {
                // SEULEMENT les grecs du modèle — tick 13 en temps réel, 83 en
                // différé. Les variantes bid/ask/last servent des SENTINELLES
                // quand la cote manque : une IV de -1, un gamma de -2. Les
                // absorber écraserait le bon calcul par un nombre négatif, et le
                // GEX qui en sortirait serait faux sans que rien ne le signale.
                if !matches!(
                    c.field,
                    TickType::ModelOption | TickType::DelayedModelOption
                ) {
                    return;
                }
                // On ne remplace jamais une valeur déjà reçue par un vide : IB
                // renvoie des calculs partiels avant le calcul complet.
                self.iv = positif(c.implied_volatility).or(self.iv);
                self.gamma = positif(c.gamma).or(self.gamma);
                self.vega = positif(c.vega).or(self.vega);
                self.sous_jacent = positif(c.underlying_price).or(self.sous_jacent);
                // Le delta d'un put est négatif mais jamais sous -1 ; le theta,
                // lui, n'a pas de plancher utile — seul un -2 exact le trahirait,
                // et le filtre sur ModelOption l'a déjà écarté.
                self.delta = signe(c.delta, -1.0).or(self.delta);
                self.theta = signe(c.theta, f64::NEG_INFINITY).or(self.theta);
            }
            TickTypes::Size(s) => {
                if matches!(
                    s.tick_type,
                    TickType::OptionCallOpenInterest
                        | TickType::OptionPutOpenInterest
                        | TickType::FuturesOpenInterest
                        | TickType::OpenInterest
                ) {
                    self.open_interest = Some(s.size);
                }
            }
            _ => {}
        }
    }
}

/// Une grandeur qui ne peut pas être négative : IV, gamma, vega, prix.
///
/// IB signale « pas de valeur » par -1 ou -2 plutôt que par un vide. Un seuil
/// unique ne convient pas à tous les champs : le theta d'une option courte vaut
/// légitimement -58, et l'écarter perdrait une donnée vraie.
fn positif(valeur: Option<f64>) -> Option<f64> {
    valeur.filter(|v| v.is_finite() && *v >= 0.0)
}

/// Une grandeur qui peut être négative, mais pas n'importe comment.
///
/// Le delta d'un put vit dans [-1, 0] et le theta peut plonger très bas. La
/// sentinelle -2 d'IB est hors du domaine du delta, et c'est le seul champ où
/// elle se confondrait avec une valeur plausible.
fn signe(valeur: Option<f64>, borne_basse: f64) -> Option<f64> {
    valeur.filter(|v| v.is_finite() && *v >= borne_basse)
}

/// Le sens d'un contrat, ou rien si ce n'en est pas un.
fn sens_de(droit: Option<ibapi::contracts::OptionRight>) -> Option<Sens> {
    use ibapi::contracts::OptionRight;
    match droit {
        Some(OptionRight::Call) => Some(Sens::Call),
        Some(OptionRight::Put) => Some(Sens::Put),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ibapi::contracts::OptionRight;

    /// Les trois codes que le sondage d'IB avait identifiés. Se tromper de code
    /// ne produirait pas une erreur mais un open interest absent, donc un GEX nul
    /// sur les strikes concernés.
    #[test]
    fn les_trois_codes_d_open_interest_sont_lus() {
        assert_eq!(TickType::OptionCallOpenInterest as i32, 27);
        assert_eq!(TickType::OptionPutOpenInterest as i32, 28);
        assert_eq!(TickType::FuturesOpenInterest as i32, 86);
    }

    /// Les ticks génériques ne coûtent aucune ligne : on demande les deux, faute
    /// de savoir lequel IB retient pour une option sur future.
    #[test]
    fn les_deux_ticks_generiques_sont_demandes() {
        assert_eq!(TICKS_GENERIQUES, ["101", "588"]);
    }

    #[test]
    fn le_sens_se_lit_sur_le_droit() {
        assert_eq!(sens_de(Some(OptionRight::Call)), Some(Sens::Call));
        assert_eq!(sens_de(Some(OptionRight::Put)), Some(Sens::Put));
        assert_eq!(sens_de(None), None);
    }

    /// IB renvoie des calculs partiels avant le calcul complet : écraser une
    /// valeur déjà reçue par un vide perdrait le gamma sur les contrats lents.
    #[test]
    fn un_tick_vide_n_efface_pas_ce_qui_est_deja_recu() {
        let mut v = Valeurs {
            gamma: Some(0.0004),
            iv: Some(0.18),
            ..Default::default()
        };
        let vide = ibapi::contracts::OptionComputation::default();
        v.absorber(&TickTypes::OptionComputation(vide));
        assert_eq!(v.gamma, Some(0.0004));
        assert_eq!(v.iv, Some(0.18));
    }

    /// Le délai de garde est mesuré, pas choisi : à quatre secondes un quart des
    /// contrats reste muet, et un contrat muet ne se distingue pas d'un contrat
    /// sans open interest.
    #[test]
    fn le_delai_de_garde_est_genereux() {
        assert!(ATTENTE_LOT >= Duration::from_secs(10));
    }
}
