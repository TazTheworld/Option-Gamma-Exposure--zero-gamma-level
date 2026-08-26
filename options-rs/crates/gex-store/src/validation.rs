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
            observations.push(Observation {
                gex: avant.gex,
                sous_zero_gamma: avant.zero_gamma.map(|z| avant.spot < z),
                // Ramené à une base journalière pour comparer des intervalles
                // inégaux : un mouvement sur trois jours n'est pas comparable à
                // un mouvement sur un jour, et la racine est la bonne échelle.
                mouvement_par_jour: rendement.abs() / ecart.sqrt(),
                call_wall_franchi: avant.call_wall.map(|m| apres.spot > m),
                put_wall_franchi: avant.put_wall.map(|m| apres.spot < m),
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

fn mediane(valeurs: &[f64]) -> f64 {
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
        assert_eq!(taux_de_franchissement(&[true, false, false, false]), Some(0.25));
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
}
