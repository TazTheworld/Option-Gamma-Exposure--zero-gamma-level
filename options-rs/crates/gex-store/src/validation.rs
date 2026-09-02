//! Le modèle tient-il ? Mesure sur l'historique accumulé.
//!
//! Le GEX avance des affirmations vérifiables, et ce module les confronte aux
//! relevés :
//!
//! 1. en gamma négatif, les mouvements sont plus amples qu'en gamma positif ;
//! 2. c'est la position vis-à-vis du zero gamma qui décide du régime, pas le
//!    signe du GEX seul ;
//! 3. le prix bute sur les murs — il les touche en séance, mais n'y clôture pas.
//!
//! Ces trois-là portent sur une **amplitude** : le gamma dit combien le prix
//! bouge, jamais vers où. Deux autres portent sur un **sens**, et c'est ce qui
//! les rend différentes à mesurer :
//!
//! 4. le charm fait fondre le delta des teneurs ; ils échangent l'inverse pour
//!    rester couverts, donc le prix devrait dériver à l'opposé de son signe ;
//! 5. la vanna fait de même à chaque mouvement de volatilité — et c'est le
//!    **produit** `vanna x variation d'IV` qui change de signe, non la vanna,
//!    dont le numérateur est l'opposé de celui du charm par construction.
//!
//! D'où [`Observation::mouvement_signe`] à côté de
//! [`Observation::mouvement_par_jour`] : mesurer une dérive sur une valeur
//! absolue effacerait exactement ce que ces deux affirmations avancent.
//!
//! La sixième porte sur une **attraction**, ce qui demande encore autre chose :
//!
//! 6. le prix est attiré vers le max pain — mais se rapprocher ne le prouve
//!    pas, puisqu'un prix qui revient vers sa moyenne se rapproche de tout
//!    niveau proche. C'est la comparaison entre les observations parties de
//!    près et celles parties de loin qui distingue, le pinning agissant
//!    localement.
//!
//! **Ce n'est pas un backtest de stratégie.** On mesure si la description du
//! terrain est exacte, pas si on peut en tirer de l'argent.
//!
//! La mesure est faite séparément pour chaque périmètre — sous-jacent, horizon,
//! source de gamma, convention de temps. Les mélanger reviendrait à classer le
//! régime d'après un GEX qui change de signe rien qu'en changeant d'horizon.

use std::collections::BTreeMap;
use std::path::Path;

use crate::historique::COLONNES;

/// Relevés en deçà desquels aucune conclusion n'est tirée.
///
/// Vingt, et ce n'est pas une politesse : sous ce seuil, l'écart mesuré tient
/// autant au hasard qu'au modèle. Le dire vaut mieux que servir un pourcentage
/// qui a l'air d'une mesure.
pub const N_MINIMAL: usize = 20;

/// Écart minimal entre deux relevés, en jours.
///
/// Deux relevés du même quart d'heure ne sont pas deux observations : le
/// mouvement entre eux est du bruit de cotation, pas un déplacement de séance.
pub const INTERVALLE_MINIMAL: f64 = 0.5;

/// Un relevé lu depuis l'historique.
#[derive(Debug, Clone, PartialEq)]
pub struct Releve {
    /// L'instant, tel qu'écrit — trié lexicographiquement, donc chronologiquement.
    pub instant: String,
    /// Le produit.
    pub ticker: String,
    /// L'horizon, « all » compris.
    pub dte_max: String,
    /// D'où venait le gamma.
    pub source_gamma: String,
    /// Comment le temps était mesuré.
    pub convention: String,
    /// Prix du sous-jacent.
    pub spot: f64,
    /// GEX total.
    pub gex: f64,
    /// Zero gamma retenu, s'il existait.
    pub zero_gamma: Option<f64>,
    /// Mur call en gamma.
    pub call_wall: Option<f64>,
    /// Mur put en gamma.
    pub put_wall: Option<f64>,
    /// Charm total, en dollars de delta par jour.
    ///
    /// Facultatif comme les autres : un historique écrit par une version
    /// antérieure n'a pas la colonne, et l'absence se dit `None` — jamais zéro,
    /// qui se lirait comme « le delta ne fond pas ».
    pub charm: Option<f64>,
    /// Vanna totale, en dollars de delta par point de volatilité.
    pub vanna: Option<f64>,
    /// Volatilité implicite à la monnaie, sur l'échéance la plus proche.
    pub iv_atm: Option<f64>,
    /// Le max pain de l'échéance la plus proche.
    pub max_pain: Option<f64>,
}

impl Releve {
    /// Ce qui rend deux relevés comparables.
    ///
    /// Mélanger l'un des quatre compare des choses différentes : un `dte_max` de
    /// 7 et de 30 peuvent donner des GEX de signes opposés, et deux conventions
    /// de temps déplacent le zero gamma de douze points.
    fn perimetre(&self) -> (String, String, String, String) {
        (
            self.ticker.clone(),
            self.dte_max.clone(),
            self.source_gamma.clone(),
            self.convention.clone(),
        )
    }

    /// Le jour civil du relevé, pour ne compter une séance qu'une fois.
    fn jour(&self) -> &str {
        self.instant.split(' ').next().unwrap_or(&self.instant)
    }
}

/// Ce qui empêche de valider.
#[derive(Debug)]
pub enum ErreurValidation {
    /// L'historique est absent ou illisible.
    Fichier(std::io::Error),
    /// L'en-tête ne correspond pas au schéma attendu.
    Schema(String),
}

impl std::fmt::Display for ErreurValidation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErreurValidation::Fichier(e) => write!(
                f,
                "Aucun historique lisible : {e}. Lance `gex NQ` au moins une \
                 fois — l'enregistrement est automatique, sauf --no-history."
            ),
            ErreurValidation::Schema(m) => write!(f, "Historique inexploitable : {m}"),
        }
    }
}

impl std::error::Error for ErreurValidation {}

/// Lit l'historique.
pub fn lire_historique(chemin: impl AsRef<Path>) -> Result<Vec<Releve>, ErreurValidation> {
    let contenu = std::fs::read_to_string(chemin.as_ref()).map_err(ErreurValidation::Fichier)?;
    let mut lignes = contenu.lines();
    let entete: Vec<&str> = lignes
        .next()
        .ok_or_else(|| ErreurValidation::Schema("fichier vide".to_string()))?
        .split(',')
        .collect();

    let index = |nom: &str| entete.iter().position(|c| *c == nom);
    // Les colonnes indispensables. Un historique écrit par une version
    // antérieure peut manquer des autres : les exiger toutes rendrait illisible
    // ce qui a déjà été accumulé.
    let (i_ts, i_ticker, i_spot, i_gex) = (
        index("timestamp"),
        index("ticker"),
        index("spot"),
        index("total_gex"),
    );
    let (Some(i_ts), Some(i_ticker), Some(i_spot), Some(i_gex)) = (i_ts, i_ticker, i_spot, i_gex)
    else {
        return Err(ErreurValidation::Schema(format!(
            "colonnes attendues absentes ; le schéma est {}",
            COLONNES.join(",")
        )));
    };

    let flottant = |champs: &[&str], i: Option<usize>| -> Option<f64> {
        i.and_then(|i| champs.get(i))
            .and_then(|v| v.trim().parse::<f64>().ok())
    };
    let texte = |champs: &[&str], i: Option<usize>| -> String {
        i.and_then(|i| champs.get(i))
            .map(|v| v.trim().to_string())
            .unwrap_or_default()
    };

    let mut releves = Vec::new();
    for ligne in lignes {
        if ligne.trim().is_empty() {
            continue;
        }
        let champs: Vec<&str> = ligne.split(',').collect();
        let (Some(spot), Some(gex)) = (
            flottant(&champs, Some(i_spot)),
            flottant(&champs, Some(i_gex)),
        ) else {
            continue;
        };
        releves.push(Releve {
            instant: champs.get(i_ts).unwrap_or(&"").trim().to_string(),
            ticker: champs.get(i_ticker).unwrap_or(&"").trim().to_string(),
            dte_max: texte(&champs, index("dte_max")),
            source_gamma: texte(&champs, index("source_gamma")),
            convention: texte(&champs, index("time_convention")),
            spot,
            gex,
            zero_gamma: flottant(&champs, index("zero_gamma")),
            call_wall: flottant(&champs, index("call_wall")),
            put_wall: flottant(&champs, index("put_wall")),
            charm: flottant(&champs, index("charm")),
            vanna: flottant(&champs, index("vanna")),
            iv_atm: flottant(&champs, index("iv_atm")),
            max_pain: flottant(&champs, index("max_pain")),
        });
    }
    Ok(releves)
}

/// Une observation : un relevé, et ce qui s'est passé ensuite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Observation {
    /// GEX au moment du relevé.
    pub gex: f64,
    /// Le spot était-il sous le zero gamma ?
    pub sous_zero_gamma: Option<bool>,
    /// Mouvement absolu, ramené à la journée.
    pub mouvement_par_jour: f64,
    /// Le prix a-t-il clôturé au-delà du call wall ?
    pub call_wall_franchi: Option<bool>,
    /// Le prix a-t-il clôturé sous le put wall ?
    pub put_wall_franchi: Option<bool>,
    /// Mouvement **signé**, ramené à la journée.
    ///
    /// Distinct de [`Observation::mouvement_par_jour`], qui est absolu. Les trois
    /// premières affirmations demandent une amplitude — le gamma dit combien ça
    /// bouge, pas dans quel sens. Le charm et la vanna, eux, imposent un sens :
    /// mesurer leur effet sur une valeur absolue le rendrait invisible.
    pub mouvement_signe: f64,
    /// Le prix est-il allé dans le sens qu'impose le charm ?
    ///
    /// Les teneurs échangent l'**inverse** de la variation de leur delta : un
    /// charm négatif fait fondre leur delta, ils achètent pour rester couverts,
    /// et cet achat pousse vers le haut. Le sens attendu est donc l'opposé du
    /// signe du charm. `None` quand le relevé n'a pas la colonne.
    pub sens_du_charm: Option<bool>,
    /// Valeur absolue du charm au moment du relevé.
    pub charm: Option<f64>,
    /// Le flux que la vanna impose : `vanna x variation de volatilité`.
    ///
    /// **C'est ce produit qui varie de signe, pas la vanna.** La vanna garde le
    /// sien par construction — son numérateur est l'opposé de celui du charm —
    /// alors que la volatilité monte et descend. Sans ce produit, l'affirmation
    /// n'aurait jamais deux côtés à comparer.
    pub flux_vanna: Option<f64>,
    /// Le prix est-il allé dans le sens qu'impose ce flux ?
    pub sens_de_la_vanna: Option<bool>,
    /// Le prix s'est-il rapproché du max pain ?
    ///
    /// **Se rapprocher ne prouve rien à soi seul.** Un prix qui revient vers sa
    /// moyenne se rapproche de tout niveau proche de lui, max pain compris.
    /// C'est la comparaison entre les observations parties de loin et celles
    /// parties de près qui distingue un effet d'attraction d'un simple retour :
    /// le pinning agit localement, donc il devrait se voir surtout de près.
    pub vers_le_max_pain: Option<bool>,
    /// Distance de départ au max pain, en fraction du spot.
    pub distance_max_pain: Option<f64>,
}

/// Apparie chaque relevé au suivant du même périmètre.
///
/// Une seule séance compte une fois : deux exécutions du même jour ne sont pas
/// deux observations, et les compter double gonflerait artificiellement
/// l'échantillon dans le régime qui a duré le plus longtemps.
pub fn observer(releves: &[Releve], intervalle_minimal: f64) -> Vec<Observation> {
    let mut par_perimetre: BTreeMap<(String, String, String, String), Vec<&Releve>> =
        BTreeMap::new();
    for r in releves {
        par_perimetre.entry(r.perimetre()).or_default().push(r);
    }

    let mut observations = Vec::new();
    for (_, mut groupe) in par_perimetre {
        groupe.sort_by(|a, b| a.instant.cmp(&b.instant));
        // Le DERNIER relevé de chaque journée : c'est celui dont le contexte est
        // le plus complet, la séance étant la plus avancée.
        let mut par_jour: BTreeMap<&str, &Releve> = BTreeMap::new();
        for r in groupe {
            par_jour.insert(r.jour(), r);
        }
        let jours: Vec<&Releve> = par_jour.into_values().collect();

        for paire in jours.windows(2) {
            let (avant, apres) = (paire[0], paire[1]);
            let ecart = jours_entre(&avant.instant, &apres.instant);
            if ecart < intervalle_minimal || avant.spot <= 0.0 {
                continue;
            }
            let rendement = (apres.spot - avant.spot) / avant.spot;
            // Le flux de vanna demande les DEUX bouts : sans volatilité au
            // départ et à l'arrivée, il n'y a pas de variation à multiplier.
            let flux_vanna = match (avant.vanna, avant.iv_atm, apres.iv_atm) {
                (Some(v), Some(iv0), Some(iv1)) => Some(v * (iv1 - iv0)),
                _ => None,
            };
            observations.push(Observation {
                gex: avant.gex,
                sous_zero_gamma: avant.zero_gamma.map(|z| avant.spot < z),
                // Ramené à une base journalière pour comparer des intervalles
                // inégaux : un mouvement sur trois jours n'est pas comparable à
                // un mouvement sur un jour, et la racine est la bonne échelle.
                mouvement_par_jour: rendement.abs() / ecart.sqrt(),
                mouvement_signe: rendement / ecart.sqrt(),
                call_wall_franchi: avant.call_wall.map(|m| apres.spot > m),
                put_wall_franchi: avant.put_wall.map(|m| apres.spot < m),
                // Un rendement nul ne va dans aucun sens : le compter comme
                // conforme gonflerait le taux d'un cas qui ne dit rien.
                sens_du_charm: avant
                    .charm
                    .filter(|c| *c != 0.0 && rendement != 0.0)
                    .map(|c| rendement.signum() == -c.signum()),
                charm: avant.charm.map(f64::abs),
                sens_de_la_vanna: flux_vanna
                    .filter(|f| *f != 0.0 && rendement != 0.0)
                    .map(|f| rendement.signum() == -f.signum()),
                flux_vanna,
                // Le max pain de DÉPART, jamais celui d'arrivée : c'est le
                // niveau que l'on connaissait avant le mouvement. Le juger sur
                // celui qu'on découvre après reviendrait à prédire le passé.
                vers_le_max_pain: avant
                    .max_pain
                    .map(|m| (apres.spot - m).abs() < (avant.spot - m).abs()),
                distance_max_pain: avant.max_pain.map(|m| (avant.spot - m).abs() / avant.spot),
            });
        }
    }
    observations
}

/// Jours entre deux horodatages `AAAA-MM-JJ HH:MM`.
fn jours_entre(avant: &str, apres: &str) -> f64 {
    let lire = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").ok();
    match (lire(avant), lire(apres)) {
        (Some(a), Some(b)) => (b - a).num_seconds() as f64 / 86_400.0,
        _ => 0.0,
    }
}

/// Le résultat d'une comparaison entre deux groupes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Comparaison {
    /// Relevés dans le premier groupe.
    pub n_a: usize,
    /// Relevés dans le second.
    pub n_b: usize,
    /// Médiane du premier.
    pub mediane_a: f64,
    /// Médiane du second.
    pub mediane_b: f64,
    /// Écart relatif, ou rien s'il n'est pas calculable.
    pub ecart: Option<f64>,
    /// L'échantillon suffit-il pour conclure ?
    pub concluant: bool,
}

/// Compare l'amplitude des mouvements entre deux groupes.
pub fn comparer(a: &[f64], b: &[f64]) -> Comparaison {
    let (ma, mb) = (mediane(a), mediane(b));
    // Un spot figé d'un relevé à l'autre donne une médiane nulle : le rapport
    // n'existe pas, et le NaN qui en sortirait se lirait comme « contraire au
    // modèle » alors qu'il ne dit rien.
    let ecart = if a.len() >= 3 && b.len() >= 3 && mb != 0.0 {
        Some(ma / mb - 1.0)
    } else {
        None
    };
    Comparaison {
        n_a: a.len(),
        n_b: b.len(),
        mediane_a: ma,
        mediane_b: mb,
        ecart,
        concluant: a.len() + b.len() >= N_MINIMAL,
    }
}

/// Ce que deux groupes donnent en dérive **signée**.
///
/// Type distinct de [`Comparaison`], et non par goût de la symétrie : celle-ci
/// rend un rapport, ce qui n'a aucun sens sur des valeurs signées. Deux dérives
/// de signes opposés donneraient un quotient négatif qu'on ne saurait pas
/// distinguer d'une simple réduction, et une médiane proche de zéro le ferait
/// exploser. Ici l'écart se dit **par soustraction**, en points de pourcentage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Derive {
    /// Effectif du premier groupe.
    pub n_a: usize,
    /// Effectif du second.
    pub n_b: usize,
    /// Dérive médiane du premier groupe.
    pub mediane_a: f64,
    /// Dérive médiane du second.
    pub mediane_b: f64,
    /// `mediane_a - mediane_b`. `None` sous trois observations d'un côté.
    pub ecart: Option<f64>,
    /// L'échantillon total suffit-il à conclure ?
    pub concluant: bool,
}

/// Compare la dérive signée de deux groupes.
pub fn comparer_derive(a: &[f64], b: &[f64]) -> Derive {
    let (ma, mb) = (mediane(a), mediane(b));
    Derive {
        n_a: a.len(),
        n_b: b.len(),
        mediane_a: ma,
        mediane_b: mb,
        ecart: (a.len() >= 3 && b.len() >= 3).then_some(ma - mb),
        concluant: a.len() + b.len() >= N_MINIMAL,
    }
}

/// La médiane d'un échantillon, ou zéro s'il est vide.
///
/// Publique parce que le lecteur en a besoin pour couper un échantillon en deux
/// à sa propre médiane — séparer les relevés à fort charm de ceux à charm faible
/// demande de savoir où est le milieu.
pub fn mediane(valeurs: &[f64]) -> f64 {
    if valeurs.is_empty() {
        return 0.0;
    }
    let mut tries = valeurs.to_vec();
    tries.sort_by(|x, y| x.partial_cmp(y).expect("mesure finie"));
    let milieu = tries.len() / 2;
    if tries.len().is_multiple_of(2) {
        (tries[milieu - 1] + tries[milieu]) / 2.0
    } else {
        tries[milieu]
    }
}

/// Part des cas où le mur a été franchi.
pub fn taux_de_franchissement(cas: &[bool]) -> Option<f64> {
    if cas.is_empty() {
        return None;
    }
    Some(cas.iter().filter(|f| **f).count() as f64 / cas.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn releve(instant: &str, spot: f64, gex: f64) -> Releve {
        Releve {
            instant: instant.to_string(),
            ticker: "NQ".to_string(),
            dte_max: "30".to_string(),
            source_gamma: "iv".to_string(),
            convention: "heures".to_string(),
            spot,
            gex,
            zero_gamma: Some(29_400.0),
            call_wall: Some(29_600.0),
            put_wall: Some(29_000.0),
            charm: Some(-154_030_000.0),
            vanna: Some(9_100_000.0),
            iv_atm: Some(0.184),
            max_pain: Some(29_850.0),
        }
    }

    /// Deux exécutions du même jour ne sont pas deux observations : les compter
    /// double gonflerait l'échantillon dans le régime qui a duré le plus longtemps.
    #[test]
    fn une_seance_ne_compte_qu_une_fois() {
        let releves = vec![
            releve("2026-08-24 14:00", 29_000.0, -1e8),
            releve("2026-08-24 20:00", 29_050.0, -1e8),
            releve("2026-08-25 20:00", 29_300.0, -1e8),
            releve("2026-08-26 20:00", 29_200.0, 1e8),
        ];
        let obs = observer(&releves, INTERVALLE_MINIMAL);
        assert_eq!(obs.len(), 2, "trois journées distinctes, donc deux paires");
    }

    /// Un périmètre différent n'est pas comparable : un dte_max de 7 et de 30
    /// peuvent donner des GEX de signes opposés.
    #[test]
    fn les_perimetres_ne_se_melangent_pas() {
        let mut autre = releve("2026-08-25 20:00", 29_300.0, -1e8);
        autre.dte_max = "7".to_string();
        let releves = vec![
            releve("2026-08-24 20:00", 29_000.0, -1e8),
            autre,
            releve("2026-08-26 20:00", 29_200.0, -1e8),
        ];
        let obs = observer(&releves, INTERVALLE_MINIMAL);
        // Le dte_max 30 fournit une paire ; le dte_max 7 est seul, donc aucune.
        assert_eq!(obs.len(), 1);
    }

    /// Deux relevés du même quart d'heure : le mouvement entre eux est du bruit
    /// de cotation, pas un déplacement de séance.
    #[test]
    fn deux_releves_trop_proches_ne_font_pas_une_observation() {
        let releves = vec![
            releve("2026-08-24 20:00", 29_000.0, -1e8),
            releve("2026-08-24 20:15", 29_010.0, -1e8),
        ];
        assert!(observer(&releves, INTERVALLE_MINIMAL).is_empty());
    }

    /// Le mouvement est ramené à la journée : sur trois jours il n'est pas
    /// comparable à un mouvement sur un jour.
    #[test]
    fn le_mouvement_est_ramene_a_la_journee() {
        let releves = vec![
            releve("2026-08-24 20:00", 29_000.0, -1e8),
            releve("2026-08-28 20:00", 29_290.0, -1e8),
        ];
        let obs = observer(&releves, INTERVALLE_MINIMAL);
        // 1 % sur quatre jours -> 1 % / 2
        assert!((obs[0].mouvement_par_jour - 0.005).abs() < 1e-9);
    }

    /// Un spot figé donne une médiane nulle, donc un rapport indéfini. Le NaN qui
    /// en sortirait se lirait comme « contraire au modèle » alors qu'il ne dit rien.
    #[test]
    fn un_rapport_indefini_est_dit_plutot_que_calcule() {
        let c = comparer(&[0.01, 0.02, 0.03], &[0.0, 0.0, 0.0]);
        assert_eq!(c.ecart, None);
    }

    #[test]
    fn un_echantillon_trop_court_n_est_pas_concluant() {
        let c = comparer(&[0.02; 5], &[0.01; 5]);
        assert!((c.ecart.unwrap() - 1.0).abs() < 1e-12);
        assert!(!c.concluant, "dix relevés ne concluent pas");
        let long = comparer(&[0.02; 15], &[0.01; 15]);
        assert!(long.concluant);
    }

    #[test]
    fn moins_de_trois_releves_ne_se_comparent_pas() {
        assert_eq!(comparer(&[0.02, 0.03], &[0.01; 5]).ecart, None);
    }

    #[test]
    fn le_taux_de_franchissement_se_mesure() {
        assert_eq!(
            taux_de_franchissement(&[true, false, false, false]),
            Some(0.25)
        );
        assert_eq!(taux_de_franchissement(&[]), None);
    }

    #[test]
    fn l_historique_se_relit() {
        let d = std::env::temp_dir().join("gex-test-validation");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let chemin = d.join("history.csv");

        let ligne = crate::historique::LigneHistorique {
            instant: chrono::NaiveDateTime::parse_from_str(
                "2026-08-26 06:32:12",
                "%Y-%m-%d %H:%M:%S",
            )
            .unwrap(),
            ticker: "NQ".to_string(),
            dte_max: Some(30),
            source_gamma: "iv".to_string(),
            convention: "heures".to_string(),
            spot: 29_218.5,
            gex: -478_840_000.0,
            zero_gamma: None,
            call_wall: None,
            put_wall: Some(29_050.0),
            call_wall_oi: None,
            put_wall_oi: Some(29_000.0),
            charm: -154_030_000.0,
            vanna: 9_100_000.0,
            iv_atm: Some(0.184),
            max_pain: Some(29_850.0),
            strikes: 71,
            echeances: 2,
        };
        crate::historique::enregistrer(&chemin, &ligne).unwrap();

        let relus = lire_historique(&chemin).unwrap();
        assert_eq!(relus.len(), 1);
        assert_eq!(relus[0].ticker, "NQ");
        assert!((relus[0].spot - 29_218.5).abs() < 1e-9);
        assert_eq!(relus[0].zero_gamma, None);
        assert_eq!(relus[0].put_wall, Some(29_050.0));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn un_historique_absent_nomme_ce_qui_l_ecrit() {
        let e = lire_historique("nulle-part/history.csv").unwrap_err();
        assert!(e.to_string().contains("gex NQ"), "{e}");
    }

    // ---=== Le sens : charm et vanna ===---

    /// Deux relevés d'un jour d'écart, dont on choisit le charm, la vanna et la
    /// volatilité aux deux bouts.
    fn paire(spot0: f64, spot1: f64, charm: f64, vanna: f64, iv0: f64, iv1: f64) -> Observation {
        let mut a = releve("2026-08-24 20:00", spot0, -1e8);
        let mut b = releve("2026-08-25 20:00", spot1, -1e8);
        a.charm = Some(charm);
        a.vanna = Some(vanna);
        a.iv_atm = Some(iv0);
        b.iv_atm = Some(iv1);
        observer(&[a, b], INTERVALLE_MINIMAL)[0]
    }

    /// Les teneurs échangent l'inverse de la variation de leur delta : un charm
    /// négatif les fait acheter, ce qui pousse vers le haut. Le sens attendu est
    /// donc l'opposé du signe du charm.
    #[test]
    fn le_sens_attendu_du_charm_est_l_oppose_de_son_signe() {
        // charm négatif, le prix monte : conforme.
        assert_eq!(
            paire(29_000.0, 29_300.0, -1e8, 9e6, 0.18, 0.18).sens_du_charm,
            Some(true)
        );
        // charm négatif, le prix baisse : contraire.
        assert_eq!(
            paire(29_000.0, 28_700.0, -1e8, 9e6, 0.18, 0.18).sens_du_charm,
            Some(false)
        );
        // charm positif, le prix baisse : conforme à nouveau.
        assert_eq!(
            paire(29_000.0, 28_700.0, 1e8, -9e6, 0.18, 0.18).sens_du_charm,
            Some(true)
        );
    }

    /// Un prix figé ne va dans aucun sens. Le compter comme conforme gonflerait
    /// le taux d'un cas qui ne dit rien.
    #[test]
    fn un_prix_fige_ne_compte_dans_aucun_sens() {
        let o = paire(29_000.0, 29_000.0, -1e8, 9e6, 0.18, 0.20);
        assert_eq!(o.sens_du_charm, None);
        assert_eq!(o.sens_de_la_vanna, None);
    }

    /// **Ce qui rend l'affirmation sur la vanna concluante.** La vanna garde son
    /// signe par construction — son numérateur est l'opposé de celui du charm —
    /// mais la volatilité monte et descend, donc le produit change de signe. Sans
    /// ça, il n'y aurait jamais deux côtés à comparer.
    #[test]
    fn le_flux_de_vanna_change_de_signe_avec_la_volatilite() {
        let vol_monte = paire(29_000.0, 29_300.0, -1e8, 9e6, 0.18, 0.20);
        let vol_baisse = paire(29_000.0, 29_300.0, -1e8, 9e6, 0.20, 0.18);
        let (Some(f1), Some(f2)) = (vol_monte.flux_vanna, vol_baisse.flux_vanna) else {
            panic!("les deux flux doivent exister");
        };
        assert!(f1 > 0.0 && f2 < 0.0, "flux {f1} et {f2}");
        // Et le même mouvement de prix se lit donc à l'opposé selon le sens de
        // la volatilité : c'est exactement ce qui rend le test discriminant.
        assert_eq!(vol_monte.sens_de_la_vanna, Some(false));
        assert_eq!(vol_baisse.sens_de_la_vanna, Some(true));
    }

    /// Sans volatilité aux deux bouts, il n'y a pas de variation à multiplier.
    /// L'absence se dit `None`, jamais zéro — zéro voudrait dire « la
    /// volatilité n'a pas bougé », ce qui est une mesure et non un manque.
    #[test]
    fn sans_volatilite_aux_deux_bouts_il_n_y_a_pas_de_flux() {
        let mut a = releve("2026-08-24 20:00", 29_000.0, -1e8);
        let b = releve("2026-08-25 20:00", 29_300.0, -1e8);
        a.iv_atm = None;
        let o = observer(&[a, b], INTERVALLE_MINIMAL)[0];
        assert_eq!(o.flux_vanna, None);
        assert_eq!(o.sens_de_la_vanna, None);
        // Le charm, lui, n'a pas besoin de la volatilité.
        assert!(o.sens_du_charm.is_some());
    }

    /// Le gamma dit combien le prix bouge, le charm vers où. Mesurer le second
    /// sur une valeur absolue rendrait son effet invisible.
    #[test]
    fn le_mouvement_signe_garde_le_sens_que_l_absolu_efface() {
        let hausse = paire(29_000.0, 29_300.0, -1e8, 9e6, 0.18, 0.18);
        let baisse = paire(29_000.0, 28_700.0, -1e8, 9e6, 0.18, 0.18);
        assert!(hausse.mouvement_signe > 0.0);
        assert!(baisse.mouvement_signe < 0.0);
        // L'amplitude, elle, ne les distingue pas.
        assert!(hausse.mouvement_par_jour > 0.0 && baisse.mouvement_par_jour > 0.0);
    }

    /// Un rapport n'a pas de sens sur des dérives signées : deux dérives
    /// opposées donneraient un quotient négatif qu'on confondrait avec une
    /// simple réduction. L'écart se dit par soustraction.
    #[test]
    fn la_derive_se_compare_par_soustraction() {
        let d = comparer_derive(&[0.02, 0.03, 0.04], &[-0.01, -0.02, -0.03]);
        assert_eq!(d.mediane_a, 0.03);
        assert_eq!(d.mediane_b, -0.02);
        assert_eq!(d.ecart, Some(0.05));
        // Sous trois de chaque côté, aucun écart n'est rendu.
        assert_eq!(comparer_derive(&[0.02], &[-0.01]).ecart, None);
    }

    // ---=== Le max pain ===---

    /// Deux relevés d'un jour d'écart, dont on choisit le max pain de départ.
    fn paire_mp(spot0: f64, spot1: f64, max_pain: Option<f64>) -> Observation {
        let mut a = releve("2026-08-24 20:00", spot0, -1e8);
        let b = releve("2026-08-25 20:00", spot1, -1e8);
        a.max_pain = max_pain;
        observer(&[a, b], INTERVALLE_MINIMAL)[0]
    }

    #[test]
    fn le_rapprochement_du_max_pain_se_mesure_dans_les_deux_sens() {
        // Le prix part sous le max pain et monte vers lui.
        assert_eq!(
            paire_mp(29_000.0, 29_500.0, Some(29_850.0)).vers_le_max_pain,
            Some(true)
        );
        // Il part sous et s'en éloigne encore.
        assert_eq!(
            paire_mp(29_000.0, 28_500.0, Some(29_850.0)).vers_le_max_pain,
            Some(false)
        );
        // Et par au-dessus : le rapprochement n'a pas de côté privilégié.
        assert_eq!(
            paire_mp(30_000.0, 29_900.0, Some(29_850.0)).vers_le_max_pain,
            Some(true)
        );
    }

    /// C'est le max pain de DÉPART qui compte. Juger sur celui qu'on découvre
    /// après le mouvement reviendrait à prédire le passé.
    #[test]
    fn c_est_le_max_pain_d_avant_qui_est_retenu() {
        let mut a = releve("2026-08-24 20:00", 29_000.0, -1e8);
        let mut b = releve("2026-08-25 20:00", 29_500.0, -1e8);
        a.max_pain = Some(29_850.0);
        // Un max pain d'arrivée qui rendrait le verdict opposé s'il était lu.
        b.max_pain = Some(28_000.0);
        assert_eq!(
            observer(&[a, b], INTERVALLE_MINIMAL)[0].vers_le_max_pain,
            Some(true)
        );
    }

    /// La distance de départ est ce qui permet de distinguer une attraction
    /// d'un simple retour à la moyenne : le pinning agit localement.
    #[test]
    fn la_distance_de_depart_est_relative_au_spot() {
        let o = paire_mp(29_000.0, 29_500.0, Some(29_290.0));
        let d = o.distance_max_pain.unwrap();
        assert!((d - 0.01).abs() < 1e-9, "distance {d} au lieu de 1 %");
    }

    #[test]
    fn sans_max_pain_il_n_y_a_rien_a_mesurer() {
        let o = paire_mp(29_000.0, 29_500.0, None);
        assert_eq!(o.vers_le_max_pain, None);
        assert_eq!(o.distance_max_pain, None);
    }

    #[test]
    fn la_mediane_coupe_l_echantillon_en_deux() {
        assert_eq!(mediane(&[1.0, 2.0, 3.0]), 2.0);
        assert_eq!(mediane(&[1.0, 2.0, 3.0, 4.0]), 2.5);
        assert_eq!(mediane(&[]), 0.0);
    }
}
