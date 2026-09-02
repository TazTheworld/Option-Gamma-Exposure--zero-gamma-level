//! Des contrats et des valeurs, vers une chaîne exploitable.
//!
//! Deux fonctions pures, testées sans TWS. C'est ici que le format « un contrat
//! par ligne » d'IB devient le format « un strike par ligne, deux côtés » que le
//! moteur attend — et le passage n'est pas anodin : le `conId` n'y survit pas,
//! puisqu'il y en aurait deux par ligne.

use std::collections::HashMap;

use gex_core::black76::{self, Sens};
use gex_core::chaine::{Chaine, ChaineInvalide, Cote, Ligne};
use gex_core::temps::{Convention, EcheanceNy, InstantReleve, temps_restant};

use crate::client::Valeurs;
use crate::decisions::ContratOption;

/// Au-delà de cette valeur, une volatilité implicite est une échelle et non un
/// régime : 18 ne peut pas être 1 800 % de volatilité.
const SEUIL_ECHELLE_IV: f64 = 3.0;

/// Normalise l'IV servie par IB.
///
/// IB la publie tantôt en décimal, tantôt en pourcentage. Se tromper d'échelle ne
/// produit pas une erreur mais un gamma cent fois trop petit — un nombre
/// plausible, et personne ne verrait rien.
fn iv_normalisee(brute: f64) -> f64 {
    if brute > SEUIL_ECHELLE_IV {
        brute / 100.0
    } else {
        brute
    }
}

/// Assemble les contrats énumérés et les valeurs reçues en une chaîne.
///
/// Le prix du future n'a pas à être cherché : IB le sert dans `undPrice`, donc
/// dans **chaque tick d'option**. On prend la médiane des valeurs vues plutôt
/// qu'une seule, parce qu'un contrat isolé peut servir une valeur périmée sans
/// que rien ne le distingue.
///
/// Le gamma publié par IB est gardé tel quel ; là où il manque, Black-76 le
/// retrouve depuis l'IV. Sans ce repli, le GEX au gamma publié et le profil ne
/// porteraient pas sur le même périmètre de strikes.
pub fn build_chain(
    contrats: &[ContratOption],
    valeurs: &HashMap<i32, Valeurs>,
    releve: InstantReleve,
    prix_impose: Option<f64>,
) -> Result<Chaine, ChaineInvalide> {
    let spot = match prix_impose {
        Some(p) => p,
        None => mediane(
            &valeurs
                .values()
                .filter_map(|v| v.sous_jacent)
                .filter(|p| *p > 0.0)
                .collect::<Vec<_>>(),
        )
        .ok_or(ChaineInvalide::SpotInvalide)?,
    };

    // Une entrée par (échéance, strike) : les deux côtés s'y rejoignent.
    let mut par_strike: HashMap<(EcheanceNy, u64), Ligne> = HashMap::new();
    for c in contrats {
        let cle = (c.cle.echeance, c.cle.strike.to_bits());
        let ligne = par_strike.entry(cle).or_insert(Ligne {
            echeance: c.cle.echeance,
            strike: c.cle.strike,
            call: Cote::default(),
            put: Cote::default(),
        });
        let Some(v) = valeurs.get(&c.con_id) else {
            continue;
        };
        let cote = match c.cle.sens {
            Sens::Call => &mut ligne.call,
            Sens::Put => &mut ligne.put,
        };
        cote.iv = v.iv.map(iv_normalisee).unwrap_or(0.0);
        cote.gamma = v.gamma.unwrap_or(0.0);
        cote.delta = v.delta.unwrap_or(0.0);
        cote.vega = v.vega.unwrap_or(0.0);
        cote.theta = v.theta.unwrap_or(0.0);
        cote.open_interest = v.open_interest.unwrap_or(0.0);
        cote.volume = v.volume.unwrap_or(0.0);
    }

    let mut lignes: Vec<Ligne> = par_strike.into_values().collect();
    for l in &mut lignes {
        let t = temps_restant(l.echeance, releve, Convention::Heures);
        for (sens, cote) in [(Sens::Call, &mut l.call), (Sens::Put, &mut l.put)] {
            if cote.gamma == 0.0 && cote.iv > 0.0 {
                let _ = sens;
                cote.gamma = black76::gamma(spot, l.strike, cote.iv, t, 0.0);
            }
        }
    }
    Chaine::nouvelle(lignes, spot, releve)
}

/// La médiane, ou rien si l'échantillon est vide.
fn mediane(valeurs: &[f64]) -> Option<f64> {
    if valeurs.is_empty() {
        return None;
    }
    let mut tries = valeurs.to_vec();
    tries.sort_by(|a, b| a.partial_cmp(b).expect("prix fini"));
    let milieu = tries.len() / 2;
    Some(if tries.len().is_multiple_of(2) {
        (tries[milieu - 1] + tries[milieu]) / 2.0
    } else {
        tries[milieu]
    })
}

/// Fusionne le socle et le vif.
///
/// **L'open interest vient du socle et de nulle part ailleurs.** Le figer en
/// séance n'est pas une approximation : la chambre de compensation le calcule
/// après la clôture et ne le publie qu'une fois par jour, donc la valeur du socle
/// est la seule qui existe.
///
/// Ce qui bouge vraiment en séance, c'est le prix du future et l'IV — et l'IV loin
/// de la monnaie bouge peu tout en pesant peu, le gamma s'y effondrant. Le vif ne
/// rafraîchit donc que l'IV et le gamma, là où il en a.
///
/// Un contrat du vif absent du socle est ignoré plutôt qu'ajouté : on ne lui
/// inventerait pas d'open interest, et un strike à OI nul ne contribue à rien.
pub fn fusionner(socle: &Chaine, vif: &Chaine, spot: f64) -> Result<Chaine, ChaineInvalide> {
    let frais: HashMap<(EcheanceNy, u64), &Ligne> = vif
        .lignes()
        .iter()
        .map(|l| ((l.echeance, l.strike.to_bits()), l))
        .collect();

    let lignes: Vec<Ligne> = socle
        .lignes()
        .iter()
        .map(|l| {
            let Some(neuf) = frais.get(&(l.echeance, l.strike.to_bits())) else {
                return *l;
            };
            let mut fondue = *l;
            for (ancien, nouveau) in [(&mut fondue.call, &neuf.call), (&mut fondue.put, &neuf.put)]
            {
                // Zéro veut dire « pas de donnée », pas « volatilité nulle » :
                // écraser une IV du socle par un vide du vif rendrait un gamma
                // nul sur un strike qui en porte.
                if nouveau.iv > 0.0 {
                    ancien.iv = nouveau.iv;
                }
                if nouveau.gamma > 0.0 {
                    ancien.gamma = nouveau.gamma;
                }
                // Le volume, lui, se rafraîchit — contrairement à l'open
                // interest, qu'IB ne publie qu'une fois par jour après
                // règlement. Le volume est CUMULATIF sur la séance : celui du
                // socle date du balayage, et un mur par volume figé à l'heure du
                // balayage serait faux dès midi, ce qui lui retirerait tout ce
                // qui le distingue d'un mur par open interest.
                //
                // Le maximum et non l'écrasement : le volume ne peut que croître
                // dans la journée, et un contrat dont le tick n'est pas encore
                // arrivé rend zéro. Prendre le plus grand ne peut donc rien
                // perdre, là où écraser reculerait.
                ancien.volume = ancien.volume.max(nouveau.volume);
            }
            fondue
        })
        .collect();

    Chaine::nouvelle(lignes, spot, vif.releve)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decisions::CleContrat;
    use chrono::NaiveDateTime;

    fn instant(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }
    fn ech(s: &str) -> EcheanceNy {
        EcheanceNy(instant(s))
    }
    fn releve() -> InstantReleve {
        InstantReleve(instant("2026-08-26 05:52:14"))
    }

    fn contrat(con_id: i32, strike: f64, sens: Sens) -> ContratOption {
        ContratOption {
            con_id,
            cle: CleContrat {
                echeance: ech("2026-08-27 16:00:00"),
                strike,
                sens,
            },
        }
    }

    fn valeurs_typiques() -> Valeurs {
        // Relevé chez IB le 26 août 2026, sur un call NQ à 29 410.
        Valeurs {
            iv: Some(0.222_065_285_768_969_33),
            gamma: Some(0.000_859_590_806_125_622_3),
            delta: Some(0.344_146_027_037_320_56),
            vega: Some(7.125_241_309_278_863),
            theta: Some(-58.467_787_225_682_514),
            open_interest: Some(56.0),
            volume: Some(31.0),
            sous_jacent: Some(29_233.75),
        }
    }

    #[test]
    fn les_deux_cotes_se_rejoignent_sur_un_strike() {
        let contrats = vec![
            contrat(1, 29_400.0, Sens::Call),
            contrat(2, 29_400.0, Sens::Put),
        ];
        let mut v = HashMap::new();
        v.insert(1, valeurs_typiques());
        v.insert(
            2,
            Valeurs {
                open_interest: Some(721.0),
                ..valeurs_typiques()
            },
        );
        let c = build_chain(&contrats, &v, releve(), None).unwrap();
        assert_eq!(c.lignes().len(), 1, "un seul strike, deux côtés");
        assert_eq!(c.lignes()[0].call.open_interest, 56.0);
        assert_eq!(c.lignes()[0].put.open_interest, 721.0);
    }

    /// Le prix du sous-jacent est servi gratuitement dans chaque tick : la
    /// médiane le protège d'un contrat isolé qui servirait une valeur périmée.
    #[test]
    fn le_prix_du_sous_jacent_vient_des_ticks() {
        let contrats = vec![
            contrat(1, 29_400.0, Sens::Call),
            contrat(2, 29_500.0, Sens::Call),
        ];
        let mut v = HashMap::new();
        v.insert(1, valeurs_typiques());
        v.insert(
            2,
            Valeurs {
                sous_jacent: Some(29_235.75),
                ..valeurs_typiques()
            },
        );
        let c = build_chain(&contrats, &v, releve(), None).unwrap();
        assert!((c.spot - 29_234.75).abs() < 1e-9, "spot {}", c.spot);
    }

    #[test]
    fn un_prix_impose_prime_sur_les_ticks() {
        let contrats = vec![contrat(1, 29_400.0, Sens::Call)];
        let mut v = HashMap::new();
        v.insert(1, valeurs_typiques());
        let c = build_chain(&contrats, &v, releve(), Some(30_000.0)).unwrap();
        assert_eq!(c.spot, 30_000.0);
    }

    /// Sans prix ni ticks, on refuse plutôt que d'inventer : un GEX calculé sur
    /// un spot faux reste un nombre plausible.
    #[test]
    fn sans_prix_du_tout_on_refuse() {
        let contrats = vec![contrat(1, 29_400.0, Sens::Call)];
        let vide = HashMap::new();
        assert_eq!(
            build_chain(&contrats, &vide, releve(), None).unwrap_err(),
            ChaineInvalide::SpotInvalide
        );
    }

    /// IB publie l'IV tantôt en décimal, tantôt en pourcentage. Se tromper
    /// d'échelle rendrait un gamma cent fois trop petit — un nombre plausible.
    #[test]
    fn une_iv_en_pourcentage_est_ramenee_en_decimal() {
        let contrats = vec![contrat(1, 29_400.0, Sens::Call)];
        let mut v = HashMap::new();
        v.insert(
            1,
            Valeurs {
                iv: Some(22.2),
                ..valeurs_typiques()
            },
        );
        let c = build_chain(&contrats, &v, releve(), None).unwrap();
        assert!((c.lignes()[0].call.iv - 0.222).abs() < 1e-9);
    }

    /// Là où IB ne publie pas de gamma, Black-76 le retrouve depuis l'IV. Sans ce
    /// repli, le GEX au gamma publié et le profil ne porteraient pas sur le même
    /// périmètre de strikes.
    #[test]
    fn un_gamma_absent_est_recalcule_en_black76() {
        let contrats = vec![contrat(1, 29_400.0, Sens::Call)];
        let mut v = HashMap::new();
        v.insert(
            1,
            Valeurs {
                gamma: None,
                ..valeurs_typiques()
            },
        );
        let c = build_chain(&contrats, &v, releve(), None).unwrap();
        assert!(
            c.lignes()[0].call.gamma > 0.0,
            "le gamma doit être recalculé"
        );
    }

    #[test]
    fn un_contrat_sans_reponse_reste_a_zero() {
        let contrats = vec![
            contrat(1, 29_400.0, Sens::Call),
            contrat(2, 29_500.0, Sens::Call),
        ];
        let mut v = HashMap::new();
        v.insert(1, valeurs_typiques());
        let c = build_chain(&contrats, &v, releve(), None).unwrap();
        let muet = c.lignes().iter().find(|l| l.strike == 29_500.0).unwrap();
        assert_eq!(muet.call.open_interest, 0.0);
        assert_eq!(muet.call.iv, 0.0);
    }

    // ---=== fusion ===---

    fn chaine(oi: f64, iv: f64, gamma: f64, quand: &str) -> Chaine {
        let lignes: Vec<Ligne> = [29_400.0_f64, 29_500.0]
            .iter()
            .map(|k| Ligne {
                echeance: ech("2026-08-27 16:00:00"),
                strike: *k,
                call: Cote {
                    iv,
                    gamma,
                    open_interest: oi,
                    ..Default::default()
                },
                put: Cote {
                    iv,
                    gamma,
                    open_interest: oi * 2.0,
                    ..Default::default()
                },
            })
            .collect();
        Chaine::nouvelle(lignes, 29_233.75, InstantReleve(instant(quand))).unwrap()
    }

    /// Le cœur de la fusion : l'open interest ne bouge pas en séance, parce que
    /// la chambre de compensation ne le publie qu'une fois par jour.
    #[test]
    fn l_open_interest_vient_du_socle_et_de_nulle_part_ailleurs() {
        let socle = chaine(500.0, 0.20, 0.0008, "2026-08-26 05:00:00");
        let vif = chaine(9_999.0, 0.25, 0.0009, "2026-08-26 05:52:14");
        let f = fusionner(&socle, &vif, 29_300.0).unwrap();
        assert_eq!(f.lignes()[0].call.open_interest, 500.0);
        assert_eq!(f.lignes()[0].put.open_interest, 1_000.0);
        // L'IV et le gamma, eux, viennent du vif
        assert!((f.lignes()[0].call.iv - 0.25).abs() < 1e-12);
        assert!((f.lignes()[0].call.gamma - 0.0009).abs() < 1e-12);
        // Et l'instant est celui du vif : c'est lui qui date le relevé
        assert_eq!(f.releve.0, instant("2026-08-26 05:52:14"));
        assert_eq!(f.spot, 29_300.0);
    }

    /// Zéro veut dire « pas de donnée », pas « volatilité nulle » : écraser l'IV
    /// du socle par un vide du vif rendrait un gamma nul sur un strike qui en porte.
    #[test]
    fn un_vif_sans_donnee_n_efface_pas_le_socle() {
        let socle = chaine(500.0, 0.20, 0.0008, "2026-08-26 05:00:00");
        let vif = chaine(500.0, 0.0, 0.0, "2026-08-26 05:52:14");
        let f = fusionner(&socle, &vif, 29_300.0).unwrap();
        assert!((f.lignes()[0].call.iv - 0.20).abs() < 1e-12);
        assert!((f.lignes()[0].call.gamma - 0.0008).abs() < 1e-12);
    }

    /// Le vif peut porter un strike que le socle n'avait pas : on ne l'invente
    /// pas, faute d'open interest à lui donner.
    #[test]
    fn un_strike_absent_du_socle_est_ignore() {
        let socle = chaine(500.0, 0.20, 0.0008, "2026-08-26 05:00:00");
        let mut lignes = vif_avec_strike_inconnu();
        lignes.sort_by(|a, b| a.strike.partial_cmp(&b.strike).unwrap());
        let vif = Chaine::nouvelle(
            lignes,
            29_233.75,
            InstantReleve(instant("2026-08-26 05:52:14")),
        )
        .unwrap();
        let f = fusionner(&socle, &vif, 29_300.0).unwrap();
        assert_eq!(f.lignes().len(), socle.lignes().len());
        assert!(f.lignes().iter().all(|l| l.strike != 99_999.0));
    }

    /// Le volume se rafraîchit depuis le vif, contrairement à l'open interest.
    ///
    /// L'open interest n'est publié qu'une fois par jour : le garder du socle est
    /// juste. Le volume est cumulatif sur la séance — figé à l'heure du balayage,
    /// il serait faux dès midi, et un mur par volume n'aurait plus rien qui le
    /// distingue d'un mur par open interest.
    #[test]
    fn le_volume_se_rafraichit_mais_pas_l_open_interest() {
        let ligne = |volume: f64, oi: f64| Ligne {
            echeance: EcheanceNy(instant("2026-08-27 16:00:00")),
            strike: 29_300.0,
            call: Cote {
                iv: 0.2,
                gamma: 1e-5,
                open_interest: oi,
                volume,
                ..Default::default()
            },
            put: Cote::default(),
        };
        let socle = Chaine::nouvelle(
            vec![ligne(40.0, 500.0)],
            29_300.0,
            InstantReleve(instant("2026-08-27 05:00:00")),
        )
        .unwrap();
        let vif = Chaine::nouvelle(
            vec![ligne(310.0, 0.0)],
            29_300.0,
            InstantReleve(instant("2026-08-27 12:00:00")),
        )
        .unwrap();

        let f = fusionner(&socle, &vif, 29_300.0).unwrap();
        assert_eq!(
            f.lignes()[0].call.volume,
            310.0,
            "le volume doit suivre le vif"
        );
        assert_eq!(
            f.lignes()[0].call.open_interest,
            500.0,
            "l'open interest reste celui du socle"
        );
    }

    /// Un tick de volume pas encore arrivé rend zéro. Écraser ferait reculer un
    /// compteur qui ne peut que croître ; on prend donc le plus grand.
    #[test]
    fn un_volume_absent_du_vif_ne_fait_pas_reculer_le_compteur() {
        let ligne = |volume: f64| Ligne {
            echeance: EcheanceNy(instant("2026-08-27 16:00:00")),
            strike: 29_300.0,
            call: Cote {
                iv: 0.2,
                gamma: 1e-5,
                volume,
                ..Default::default()
            },
            put: Cote::default(),
        };
        let socle = Chaine::nouvelle(
            vec![ligne(310.0)],
            29_300.0,
            InstantReleve(instant("2026-08-27 05:00:00")),
        )
        .unwrap();
        let vif = Chaine::nouvelle(
            vec![ligne(0.0)],
            29_300.0,
            InstantReleve(instant("2026-08-27 12:00:00")),
        )
        .unwrap();
        let f = fusionner(&socle, &vif, 29_300.0).unwrap();
        assert_eq!(f.lignes()[0].call.volume, 310.0);
    }

    fn vif_avec_strike_inconnu() -> Vec<Ligne> {
        vec![
            Ligne {
                echeance: ech("2026-08-27 16:00:00"),
                strike: 99_999.0,
                call: Cote {
                    iv: 0.9,
                    gamma: 0.9,
                    ..Default::default()
                },
                put: Cote::default(),
            },
            Ligne {
                echeance: ech("2026-08-27 16:00:00"),
                strike: 29_400.0,
                call: Cote {
                    iv: 0.25,
                    gamma: 0.0009,
                    ..Default::default()
                },
                put: Cote::default(),
            },
        ]
    }
}
