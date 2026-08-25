//! Le format pivot : ce qu'une chaîne d'options est, une fois lue.
//!
//! Le moteur Python portait ce format en colonnes de DataFrame, aux noms hérités
//! du CBOE — `CallOpenInt`, `PutIV`, `StrikePrice`. Ces noms restent la vérité du
//! fichier, puisque les relevés archivés les portent et doivent rester
//! rejouables ; ils ne sont plus la vérité du calcul. Ici une ligne est une
//! ligne, et un côté un côté.
//!
//! Le gain n'est pas cosmétique. En colonnes, rien n'empêche de lire `CallIV`
//! avec `PutOpenInt` : ce sont deux vecteurs de flottants de même longueur, et
//! l'erreur produit un chiffre. Ici les deux côtés sont deux valeurs distinctes.

use crate::temps::{EcheanceNy, InstantReleve};

/// Un côté d'un strike : tout ce que la source publie pour un contrat.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Cote {
    /// Volatilité implicite, en décimal — 0,18 et non 18.
    pub iv: f64,
    /// Gamma **tel que publié** par la source. Zéro quand elle n'en publie pas.
    pub gamma: f64,
    /// Delta publié. Zéro quand absent.
    pub delta: f64,
    /// Open interest, en contrats.
    pub open_interest: f64,
    /// Vega publié. Zéro quand absent.
    pub vega: f64,
    /// Theta publié. Zéro quand absent.
    pub theta: f64,
}

impl Cote {
    /// Remplace par zéro tout ce qui n'est pas un nombre fini.
    ///
    /// Un NaN qui traverse le calcul contamine la somme entière : un seul strike
    /// mal servi suffirait à rendre le GEX total NaN, et l'affichage dirait
    /// « nan » là où il devrait dire un montant. Zéro se lit comme « pas de
    /// donnée » et ne contribue à rien.
    fn assainir(mut self) -> Self {
        for champ in [
            &mut self.iv,
            &mut self.gamma,
            &mut self.delta,
            &mut self.open_interest,
            &mut self.vega,
            &mut self.theta,
        ] {
            if !champ.is_finite() {
                *champ = 0.0;
            }
        }
        self
    }
}

/// Un strike sur une échéance, avec ses deux côtés.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ligne {
    /// L'instant de règlement, en heure de New York.
    pub echeance: EcheanceNy,
    /// Le prix d'exercice.
    pub strike: f64,
    /// Le call.
    pub call: Cote,
    /// Le put.
    pub put: Cote,
}

/// Une chaîne complète, telle qu'un relevé la porte.
#[derive(Debug, Clone, PartialEq)]
pub struct Chaine {
    /// Les lignes, triées par échéance puis par strike.
    lignes: Vec<Ligne>,
    /// Le prix du sous-jacent au moment du relevé.
    pub spot: f64,
    /// L'instant du relevé, en UTC.
    pub releve: InstantReleve,
}

/// Ce qui empêche une chaîne d'exister.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChaineInvalide {
    /// Aucune ligne exploitable après nettoyage.
    Vide,
    /// Le prix du sous-jacent est absent ou absurde.
    SpotInvalide,
}

impl std::fmt::Display for ChaineInvalide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Rendre une chaîne vide donnerait un GEX de zéro, qui est un chiffre
            // et non une erreur : personne ne verrait que la collecte a échoué.
            ChaineInvalide::Vide => write!(
                f,
                "Aucune ligne exploitable : tous les strikes sont absents ou non finis."
            ),
            ChaineInvalide::SpotInvalide => write!(
                f,
                "Prix du sous-jacent absent ou negatif : rien ne peut etre calcule sans lui."
            ),
        }
    }
}

impl std::error::Error for ChaineInvalide {}

impl Chaine {
    /// Construit une chaîne à partir de lignes brutes.
    ///
    /// Assainit chaque côté, écarte les strikes inexploitables, et trie. Le tri
    /// n'est pas cosmétique : le profil de gamma et les murs parcourent les
    /// strikes dans l'ordre, et deux relevés du même instant devraient donner
    /// exactement le même résultat.
    pub fn nouvelle(
        lignes: impl IntoIterator<Item = Ligne>,
        spot: f64,
        releve: InstantReleve,
    ) -> Result<Self, ChaineInvalide> {
        if !spot.is_finite() || spot <= 0.0 {
            return Err(ChaineInvalide::SpotInvalide);
        }
        let mut lignes: Vec<Ligne> = lignes
            .into_iter()
            .filter(|l| l.strike.is_finite() && l.strike > 0.0)
            .map(|l| Ligne {
                call: l.call.assainir(),
                put: l.put.assainir(),
                ..l
            })
            .collect();
        if lignes.is_empty() {
            return Err(ChaineInvalide::Vide);
        }
        lignes.sort_by(|a, b| {
            a.echeance
                .cmp(&b.echeance)
                // Les strikes sont finis : le tri total est garanti par le filtre.
                .then(a.strike.partial_cmp(&b.strike).expect("strike fini"))
        });
        Ok(Chaine {
            lignes,
            spot,
            releve,
        })
    }

    /// Les lignes, triées.
    pub fn lignes(&self) -> &[Ligne] {
        &self.lignes
    }

    /// Les échéances distinctes, dans l'ordre.
    pub fn echeances(&self) -> Vec<EcheanceNy> {
        let mut vues: Vec<EcheanceNy> = self.lignes.iter().map(|l| l.echeance).collect();
        vues.dedup();
        vues
    }

    /// Une chaîne restreinte aux lignes retenues, sans retrier.
    pub fn filtrer(&self, garde: impl Fn(&Ligne) -> bool) -> Result<Self, ChaineInvalide> {
        let lignes: Vec<Ligne> = self.lignes.iter().copied().filter(|l| garde(l)).collect();
        if lignes.is_empty() {
            return Err(ChaineInvalide::Vide);
        }
        Ok(Chaine {
            lignes,
            spot: self.spot,
            releve: self.releve,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;

    fn instant(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn ligne(echeance: &str, strike: f64) -> Ligne {
        Ligne {
            echeance: EcheanceNy(instant(echeance)),
            strike,
            call: Cote {
                iv: 0.18,
                open_interest: 100.0,
                ..Default::default()
            },
            put: Cote {
                iv: 0.20,
                open_interest: 200.0,
                ..Default::default()
            },
        }
    }

    fn chaine(lignes: Vec<Ligne>) -> Chaine {
        Chaine::nouvelle(lignes, 29_305.75, InstantReleve(instant("2026-08-25 20:30:16"))).unwrap()
    }

    #[test]
    fn les_lignes_sont_triees_par_echeance_puis_strike() {
        let c = chaine(vec![
            ligne("2026-08-27 16:00:00", 29_000.0),
            ligne("2026-08-26 16:00:00", 29_500.0),
            ligne("2026-08-26 16:00:00", 29_000.0),
        ]);
        let vu: Vec<(NaiveDateTime, f64)> =
            c.lignes().iter().map(|l| (l.echeance.0, l.strike)).collect();
        assert_eq!(
            vu,
            vec![
                (instant("2026-08-26 16:00:00"), 29_000.0),
                (instant("2026-08-26 16:00:00"), 29_500.0),
                (instant("2026-08-27 16:00:00"), 29_000.0),
            ]
        );
    }

    /// Un NaN qui traverse contaminerait la somme : un seul strike mal servi
    /// rendrait le GEX total « nan » au lieu d'un montant.
    #[test]
    fn les_valeurs_non_finies_deviennent_zero() {
        let mut l = ligne("2026-08-26 16:00:00", 29_000.0);
        l.call.iv = f64::NAN;
        l.put.open_interest = f64::INFINITY;
        l.call.gamma = f64::NEG_INFINITY;
        let c = chaine(vec![l]);
        let vue = c.lignes()[0];
        assert_eq!(vue.call.iv, 0.0);
        assert_eq!(vue.put.open_interest, 0.0);
        assert_eq!(vue.call.gamma, 0.0);
        // Ce qui était bon n'a pas bougé
        assert_eq!(vue.put.iv, 0.20);
    }

    #[test]
    fn un_strike_absurde_est_ecarte() {
        let c = chaine(vec![
            ligne("2026-08-26 16:00:00", 29_000.0),
            ligne("2026-08-26 16:00:00", f64::NAN),
            ligne("2026-08-26 16:00:00", -50.0),
            ligne("2026-08-26 16:00:00", 0.0),
        ]);
        assert_eq!(c.lignes().len(), 1);
    }

    /// Rendre une chaîne vide donnerait un GEX de zéro, qui est un chiffre et non
    /// une erreur : personne ne verrait que la collecte a échoué.
    #[test]
    fn une_chaine_sans_ligne_exploitable_est_refusee() {
        let vide = Chaine::nouvelle(
            vec![ligne("2026-08-26 16:00:00", f64::NAN)],
            29_000.0,
            InstantReleve(instant("2026-08-25 20:30:16")),
        );
        assert_eq!(vide.unwrap_err(), ChaineInvalide::Vide);
    }

    #[test]
    fn un_spot_absurde_est_refuse() {
        for spot in [0.0, -1.0, f64::NAN] {
            let r = Chaine::nouvelle(
                vec![ligne("2026-08-26 16:00:00", 29_000.0)],
                spot,
                InstantReleve(instant("2026-08-25 20:30:16")),
            );
            assert_eq!(r.unwrap_err(), ChaineInvalide::SpotInvalide);
        }
    }

    #[test]
    fn les_echeances_sont_distinctes_et_ordonnees() {
        let c = chaine(vec![
            ligne("2026-08-27 16:00:00", 29_000.0),
            ligne("2026-08-26 16:00:00", 29_000.0),
            ligne("2026-08-26 16:00:00", 29_500.0),
        ]);
        assert_eq!(
            c.echeances(),
            vec![
                EcheanceNy(instant("2026-08-26 16:00:00")),
                EcheanceNy(instant("2026-08-27 16:00:00")),
            ]
        );
    }

    #[test]
    fn filtrer_garde_le_spot_et_l_instant() {
        let c = chaine(vec![
            ligne("2026-08-26 16:00:00", 29_000.0),
            ligne("2026-08-27 16:00:00", 29_000.0),
        ]);
        let f = c
            .filtrer(|l| l.echeance.0 == instant("2026-08-27 16:00:00"))
            .unwrap();
        assert_eq!(f.lignes().len(), 1);
        assert_eq!(f.spot, c.spot);
        assert_eq!(f.releve, c.releve);
    }

    #[test]
    fn un_filtre_qui_ne_garde_rien_est_une_erreur() {
        let c = chaine(vec![ligne("2026-08-26 16:00:00", 29_000.0)]);
        assert_eq!(c.filtrer(|_| false).unwrap_err(), ChaineInvalide::Vide);
    }
}
