//! Les trois décisions de la boucle, sorties d'elle pour se tester sans TWS.
//!
//! Ce sont les seules qui portent un raisonnement. Le reste de la boucle est de
//! l'orchestration : demander, attendre, écrire.

use std::time::Duration as Attente;

use chrono::{Duration, NaiveDateTime};

/// Heure UTC à laquelle bascule la journée de compensation.
///
/// Le CME publie l'open interest préliminaire à 18 h heure de Chicago, soit 23 h
/// UTC en heure d'été. Après cette heure, on est déjà sur la publication du
/// lendemain.
const HEURE_OI_UTC: i64 = 23;

/// Part de la bande au-delà de laquelle le vif se resélectionne.
pub const MARGE_BANDE: f64 = 0.15;

/// La journée d'open interest à laquelle un instant appartient.
pub fn journee_compensation(instant: NaiveDateTime) -> chrono::NaiveDate {
    (instant + Duration::hours(24 - HEURE_OI_UTC)).date()
}

/// Le socle est-il périmé ?
///
/// L'open interest est calculé par la chambre de compensation après la clôture et
/// publié **une fois par jour** : rebalayer en cours de journée relirait le même
/// chiffre pour huit à treize minutes de souscriptions.
pub fn faut_il_rebalayer(dernier_socle: Option<NaiveDateTime>, maintenant: NaiveDateTime) -> bool {
    match dernier_socle {
        None => true,
        Some(avant) => journee_compensation(maintenant) > journee_compensation(avant),
    }
}

/// Les strikes extrêmes que le vif surveille.
pub fn bande_couverte(vif: &[f64]) -> Option<(f64, f64)> {
    let finis: Vec<f64> = vif.iter().copied().filter(|s| s.is_finite()).collect();
    if finis.is_empty() {
        return None;
    }
    let bas = finis.iter().copied().fold(f64::INFINITY, f64::min);
    let haut = finis.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Some((bas, haut))
}

/// Le spot approche-t-il du bord de ce que le vif surveille ?
///
/// On recycle quand le spot **approche** du bord, pas quand il en sort. Attendre
/// la sortie franche reviendrait à attendre d'être aveugle : quand le spot arrive
/// près du bord, les contrats qui portent le gamma ne sont déjà plus ceux qu'on a
/// souscrits.
pub fn faut_il_reselectionner(spot: f64, bande: Option<(f64, f64)>, marge: f64) -> bool {
    let Some((bas, haut)) = bande else {
        return true;
    };
    let largeur = haut - bas;
    if largeur <= 0.0 {
        return true;
    }
    let garde = largeur * marge;
    !(bas + garde <= spot && spot <= haut - garde)
}

/// Combien attendre avant de retenter une connexion, à la n-ième tentative.
///
/// Le redémarrage quotidien de TWS dure une poignée de secondes ; la coupure
/// hebdomadaire du dimanche, elle, attend qu'un humain se reconnecte et peut
/// durer des heures. Un délai fixe servirait mal l'un des deux : trop long pour
/// le redémarrage, trop court pour l'attente humaine, où il ferait des milliers
/// de tentatives inutiles.
///
/// D'où le doublement, plafonné : on retente vite d'abord, puis on patiente.
pub fn attente_avant_reprise(tentative: u32) -> Attente {
    const PREMIERE: u64 = 5;
    const PLAFOND: u64 = 300;
    let secondes = PREMIERE.saturating_mul(1u64 << tentative.min(6));
    Attente::from_secs(secondes.min(PLAFOND))
}

/// Le socle survit-il à une reconnexion ?
///
/// C'est tout l'intérêt de traiter la coupure comme un événement normal : le
/// socle est en mémoire ET sur disque, et l'open interest qu'il porte ne bougera
/// pas avant la publication du soir. Le rebalayer coûterait trois à cinq minutes
/// pour relire exactement les mêmes chiffres.
pub fn socle_reutilisable(
    date_socle: Option<NaiveDateTime>,
    maintenant: NaiveDateTime,
) -> bool {
    !faut_il_rebalayer(date_socle, maintenant)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instant(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    /// 23 h UTC est la bascule : le CME publie l'open interest préliminaire à
    /// 18 h Chicago, et après cette heure on est sur le chiffre du lendemain.
    #[test]
    fn la_journee_de_compensation_bascule_a_l_heure_du_cme() {
        assert_eq!(
            journee_compensation(instant("2026-08-26 22:59:00")),
            chrono::NaiveDate::parse_from_str("2026-08-26", "%Y-%m-%d").unwrap()
        );
        assert_eq!(
            journee_compensation(instant("2026-08-26 23:00:00")),
            chrono::NaiveDate::parse_from_str("2026-08-27", "%Y-%m-%d").unwrap()
        );
    }

    #[test]
    fn sans_socle_il_faut_balayer() {
        assert!(faut_il_rebalayer(None, instant("2026-08-26 05:00:00")));
    }

    /// Le point de toute la décision : ne pas payer treize minutes de
    /// souscriptions pour relire le même chiffre.
    #[test]
    fn un_socle_de_la_meme_journee_ne_se_rebalaie_pas() {
        let socle = instant("2026-08-26 05:00:00");
        assert!(!faut_il_rebalayer(Some(socle), instant("2026-08-26 12:00:00")));
        assert!(!faut_il_rebalayer(Some(socle), instant("2026-08-26 22:59:00")));
    }

    #[test]
    fn le_socle_se_rebalaie_apres_la_publication() {
        let socle = instant("2026-08-26 05:00:00");
        assert!(faut_il_rebalayer(Some(socle), instant("2026-08-26 23:00:00")));
        assert!(faut_il_rebalayer(Some(socle), instant("2026-08-27 06:00:00")));
    }

    #[test]
    fn la_bande_encadre_le_vif() {
        assert_eq!(
            bande_couverte(&[29_100.0, 29_500.0, 29_300.0]),
            Some((29_100.0, 29_500.0))
        );
        assert_eq!(bande_couverte(&[]), None);
        assert_eq!(bande_couverte(&[f64::NAN]), None);
    }

    /// On recycle AVANT la sortie franche : quand le spot arrive près du bord,
    /// les contrats qui portent le gamma ont déjà changé.
    #[test]
    fn on_recycle_quand_le_spot_approche_du_bord() {
        let bande = Some((29_000.0, 30_000.0)); // largeur 1 000, garde 150
        assert!(!faut_il_reselectionner(29_500.0, bande, MARGE_BANDE));
        assert!(!faut_il_reselectionner(29_150.0, bande, MARGE_BANDE));
        // 29 100 est DANS la bande, mais à moins de 150 du bord
        assert!(faut_il_reselectionner(29_100.0, bande, MARGE_BANDE));
        assert!(faut_il_reselectionner(30_500.0, bande, MARGE_BANDE));
    }

    /// Le redémarrage quotidien dure quelques secondes ; la coupure du dimanche
    /// attend un humain. Un délai fixe servirait mal l'un des deux.
    #[test]
    fn l_attente_double_puis_plafonne() {
        assert_eq!(attente_avant_reprise(0), Attente::from_secs(5));
        assert_eq!(attente_avant_reprise(1), Attente::from_secs(10));
        assert_eq!(attente_avant_reprise(3), Attente::from_secs(40));
        // Plafonnée : sinon l'attente du dimanche deviendrait des heures.
        assert_eq!(attente_avant_reprise(6), Attente::from_secs(300));
        assert_eq!(attente_avant_reprise(50), Attente::from_secs(300));
    }

    /// Tout l'intérêt de traiter la coupure comme un événement normal : le socle
    /// est déjà là, et l'open interest qu'il porte ne bougera pas avant le soir.
    #[test]
    fn le_socle_du_jour_survit_a_une_reconnexion() {
        let socle = instant("2026-08-26 05:00:00");
        assert!(socle_reutilisable(Some(socle), instant("2026-08-26 14:00:00")));
        // Passé la publication du CME, il est périmé et doit être rebalayé.
        assert!(!socle_reutilisable(Some(socle), instant("2026-08-27 06:00:00")));
        assert!(!socle_reutilisable(None, instant("2026-08-26 14:00:00")));
    }

    #[test]
    fn sans_bande_il_faut_selectionner() {
        assert!(faut_il_reselectionner(29_500.0, None, MARGE_BANDE));
        // Une bande dégénérée — un seul strike — n'encadre rien
        assert!(faut_il_reselectionner(
            29_500.0,
            Some((29_500.0, 29_500.0)),
            MARGE_BANDE
        ));
    }
}
