//! Le postulat du dépôt, confronté à une mesure publique.
//!
//! Tout le signe du GEX repose sur une convention : **les teneurs sont longs des
//! calls et courts des puts**. Elle n'est pas inventée ici — c'est celle de
//! Barbon & Buraschi, *Gamma Fragility* (2020) —, mais elle reste **empirique et
//! non mécanique**, et rien dans ce dépôt ne la vérifiait.
//!
//! # Ce que la CFTC permet de tester
//!
//! Le rapport *Traders in Financial Futures* ventile l'open interest par
//! catégorie de participant, dont **Dealer / Intermediary**, et paraît chaque
//! vendredi sur une photo du mardi. Il existe en deux versions : futures seuls,
//! et futures **et options** combinés — ces dernières converties en équivalent
//! futures **par leur delta**.
//!
//! Leur différence est donc le delta net du livre d'options des teneurs. Or la
//! convention en prédit le signe : un call long a un delta positif, un put vendu
//! aussi. « Longs des calls, courts des puts » implique un delta d'options
//! **positif**, et durablement.
//!
//! # Ce que ce test ne peut pas faire
//!
//! **Il ne mesure pas le gamma.** Il mesure un positionnement en delta, donc il
//! peut *affaiblir* la convention sans jamais valider le signe du GEX. Il est
//! **agrégé** — aucun signe par strike — et **hebdomadaire**, quand le GEX se
//! recalcule toutes les quinze secondes. Enfin, « Dealer / Intermediary » désigne
//! le côté vendeur au sens large, pas les seuls teneurs d'options.
//!
//! Ce module ne va rien chercher : il lit ce que `scripts/cftc-telecharger.sh` a
//! déposé. Le réseau n'a pas sa place dans une crate qui doit se tester hors
//! ligne.

use std::collections::BTreeMap;

use serde_json::Value;

/// Une ligne de rapport hebdomadaire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rapport {
    /// Le mardi photographié, en `AAAA-MM-JJ`.
    pub jour: String,
    /// Position nette des teneurs : longues moins courtes.
    pub net: i64,
    /// Open interest total du contrat.
    pub open_interest: i64,
}

/// Ce qui empêche de lire un rapport.
#[derive(Debug)]
pub enum ErreurCftc {
    /// Le fichier est absent ou illisible.
    Fichier(std::io::Error),
    /// Le contenu n'est pas le JSON attendu.
    Format(String),
}

impl std::fmt::Display for ErreurCftc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErreurCftc::Fichier(e) => write!(
                f,
                "rapport CFTC illisible : {e}. Lance `scripts/cftc-telecharger.sh` \
                 — il dépose les deux fichiers attendus."
            ),
            ErreurCftc::Format(m) => write!(f, "rapport CFTC inexploitable : {m}"),
        }
    }
}

impl std::error::Error for ErreurCftc {}

fn entier(v: &Value, cle: &str) -> Option<i64> {
    // Le service rend ses nombres en CHAÎNES. Les lire comme des nombres rendrait
    // zéro partout, ce qui se lirait comme un marché sans position.
    v.get(cle)?.as_str()?.trim().parse().ok()
}

/// Lit un rapport, en exigeant qu'il soit de la version annoncée.
///
/// `attendu` vaut `"Combined"` ou `"FutOnly"`. **Cette garde n'est pas une
/// politesse** : les deux fichiers ont exactement la même forme, et les
/// intervertir — ou passer deux fois le même — donnerait une différence nulle,
/// donc un « delta d'options toujours nul » parfaitement crédible et faux.
pub fn lire(json: &str, attendu: &str) -> Result<Vec<Rapport>, ErreurCftc> {
    let racine: Value =
        serde_json::from_str(json).map_err(|e| ErreurCftc::Format(e.to_string()))?;
    let lignes = racine
        .as_array()
        .ok_or_else(|| ErreurCftc::Format("le document n'est pas une liste".into()))?;

    let mut sortie = Vec::with_capacity(lignes.len());
    for ligne in lignes {
        let version = ligne
            .get("futonly_or_combined")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if version != attendu {
            return Err(ErreurCftc::Format(format!(
                "ce fichier est « {version} » alors que « {attendu} » était attendu : \
                 les deux versions ont la même forme, et les confondre rendrait une \
                 différence nulle qu'on prendrait pour une mesure"
            )));
        }
        let (Some(long), Some(court), Some(oi), Some(jour)) = (
            entier(ligne, "dealer_positions_long_all"),
            entier(ligne, "dealer_positions_short_all"),
            entier(ligne, "open_interest_all"),
            ligne
                .get("report_date_as_yyyy_mm_dd")
                .and_then(Value::as_str),
        ) else {
            continue;
        };
        sortie.push(Rapport {
            // L'horodatage porte une heure qui ne veut rien dire : la photo est
            // celle d'un mardi, pas d'un instant.
            jour: jour.chars().take(10).collect(),
            net: long - court,
            open_interest: oi,
        });
    }
    if sortie.is_empty() {
        return Err(ErreurCftc::Format("aucune ligne exploitable".into()));
    }
    sortie.sort_by(|a, b| a.jour.cmp(&b.jour));
    Ok(sortie)
}

/// Ce que la confrontation donne.
#[derive(Debug, Clone, PartialEq)]
pub struct Confrontation {
    /// Semaines présentes dans les deux versions.
    pub semaines: usize,
    /// La première et la dernière.
    pub debut: String,
    /// La dernière.
    pub fin: String,
    /// Semaines où le delta d'options est positif — ce que la convention prédit.
    pub conformes: usize,
    /// Semaines où il est négatif.
    pub contraires: usize,
    /// Semaines où il est exactement nul.
    pub nuls: usize,
    /// Amplitude médiane du delta d'options, en contrats.
    pub amplitude_mediane: i64,
    /// Open interest optionnel médian, en contrats.
    pub oi_optionnel_median: i64,
    /// Semaines où la position nette globale est courte.
    pub nettes_courtes: usize,
}

impl Confrontation {
    /// Part des semaines conformes à la convention.
    ///
    /// `None` si rien n'a pu être apparié : un pourcentage sur zéro semaine
    /// aurait l'air d'une mesure.
    pub fn part_conforme(&self) -> Option<f64> {
        (self.semaines > 0).then(|| self.conformes as f64 / self.semaines as f64)
    }
}

fn mediane(mut valeurs: Vec<i64>) -> i64 {
    if valeurs.is_empty() {
        return 0;
    }
    valeurs.sort_unstable();
    valeurs[valeurs.len() / 2]
}

/// Confronte les deux versions du rapport.
///
/// Seules les semaines présentes des deux côtés comptent : la différence n'a de
/// sens qu'entre deux photos du même mardi.
pub fn confronter(combine: &[Rapport], futures: &[Rapport]) -> Confrontation {
    let par_jour: BTreeMap<&str, &Rapport> = futures.iter().map(|r| (r.jour.as_str(), r)).collect();

    let (mut conformes, mut contraires, mut nuls, mut nettes_courtes) = (0, 0, 0, 0);
    let (mut amplitudes, mut ois, mut jours) = (Vec::new(), Vec::new(), Vec::new());

    for c in combine {
        let Some(f) = par_jour.get(c.jour.as_str()) else {
            continue;
        };
        let delta = c.net - f.net;
        match delta.cmp(&0) {
            std::cmp::Ordering::Greater => conformes += 1,
            std::cmp::Ordering::Less => contraires += 1,
            std::cmp::Ordering::Equal => nuls += 1,
        }
        if c.net < 0 {
            nettes_courtes += 1;
        }
        amplitudes.push(delta.abs());
        ois.push(c.open_interest - f.open_interest);
        jours.push(c.jour.clone());
    }

    Confrontation {
        semaines: jours.len(),
        debut: jours.first().cloned().unwrap_or_default(),
        fin: jours.last().cloned().unwrap_or_default(),
        conformes,
        contraires,
        nuls,
        amplitude_mediane: mediane(amplitudes),
        oi_optionnel_median: mediane(ois),
        nettes_courtes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(version: &str, lignes: &[(&str, i64, i64, i64)]) -> String {
        let corps: Vec<String> = lignes
            .iter()
            .map(|(jour, long, court, oi)| {
                format!(
                    r#"{{"report_date_as_yyyy_mm_dd":"{jour}T00:00:00.000",
                       "contract_market_name":"NASDAQ MINI",
                       "futonly_or_combined":"{version}",
                       "dealer_positions_long_all":"{long}",
                       "dealer_positions_short_all":"{court}",
                       "open_interest_all":"{oi}"}}"#
                )
            })
            .collect();
        format!("[{}]", corps.join(","))
    }

    /// Le service rend ses nombres en chaînes. Les lire comme des nombres rendrait
    /// zéro partout, ce qui se lirait comme un marché sans position.
    #[test]
    fn les_nombres_arrivent_en_chaines_et_sont_lus_quand_meme() {
        let r = lire(
            &json("Combined", &[("2026-08-25", 57875, 123505, 338076)]),
            "Combined",
        )
        .unwrap();
        assert_eq!(r[0].net, 57875 - 123505);
        assert_eq!(r[0].open_interest, 338076);
        assert_eq!(r[0].jour, "2026-08-25");
    }

    /// **La garde qui compte.** Les deux fichiers ont la même forme : les
    /// intervertir donnerait une différence nulle, donc un « delta d'options
    /// toujours nul » parfaitement crédible et faux.
    #[test]
    fn un_fichier_de_la_mauvaise_version_est_refuse() {
        let e = lire(&json("FutOnly", &[("2026-08-25", 1, 2, 3)]), "Combined").unwrap_err();
        let m = e.to_string();
        assert!(m.contains("FutOnly") && m.contains("Combined"), "{m}");
    }

    #[test]
    fn les_semaines_sortent_dans_l_ordre() {
        let r = lire(
            &json(
                "Combined",
                &[
                    ("2026-08-25", 1, 2, 9),
                    ("2026-08-11", 1, 2, 9),
                    ("2026-08-18", 1, 2, 9),
                ],
            ),
            "Combined",
        )
        .unwrap();
        let jours: Vec<&str> = r.iter().map(|x| x.jour.as_str()).collect();
        assert_eq!(jours, ["2026-08-11", "2026-08-18", "2026-08-25"]);
    }

    /// Le delta d'options est la différence entre les deux versions, et son signe
    /// est ce que la convention prédit.
    #[test]
    fn le_delta_d_options_est_la_difference_des_deux_versions() {
        let c = lire(
            &json("Combined", &[("2026-08-25", 57875, 123505, 338076)]),
            "Combined",
        )
        .unwrap();
        let f = lire(
            &json("FutOnly", &[("2026-08-25", 59255, 122503, 301987)]),
            "FutOnly",
        )
        .unwrap();
        let r = confronter(&c, &f);
        assert_eq!(r.semaines, 1);
        // (57875-123505) - (59255-122503) = -65630 + 63248 = -2382
        assert_eq!(r.amplitude_mediane, 2382);
        assert_eq!(r.contraires, 1);
        assert_eq!(r.conformes, 0);
        assert_eq!(r.oi_optionnel_median, 338076 - 301987);
        assert_eq!(r.nettes_courtes, 1);
    }

    /// Une semaine absente d'un côté n'est pas une semaine : la différence n'a de
    /// sens qu'entre deux photos du même mardi.
    #[test]
    fn seules_les_semaines_communes_comptent() {
        let c = lire(
            &json(
                "Combined",
                &[("2026-08-18", 10, 5, 100), ("2026-08-25", 10, 5, 100)],
            ),
            "Combined",
        )
        .unwrap();
        let f = lire(&json("FutOnly", &[("2026-08-25", 4, 5, 90)]), "FutOnly").unwrap();
        let r = confronter(&c, &f);
        assert_eq!(r.semaines, 1);
        assert_eq!(r.debut, "2026-08-25");
        assert_eq!(r.fin, "2026-08-25");
    }

    /// Un pourcentage sur zéro semaine aurait l'air d'une mesure.
    #[test]
    fn sans_semaine_commune_aucune_part_n_est_rendue() {
        let c = lire(&json("Combined", &[("2026-08-18", 1, 2, 9)]), "Combined").unwrap();
        let f = lire(&json("FutOnly", &[("2026-08-25", 1, 2, 9)]), "FutOnly").unwrap();
        assert_eq!(confronter(&c, &f).part_conforme(), None);
    }

    #[test]
    fn un_document_vide_est_refuse_plutot_que_rendu_vide() {
        assert!(lire("[]", "Combined").is_err());
        assert!(lire("pas du json", "Combined").is_err());
    }
}
