//! Le temps restant jusqu'à l'échéance, et les deux conventions pour le mesurer.
//!
//! C'est la partie du calcul qui s'est révélée fausse en production, et le
//! commentaire vaut d'être gardé : tant que l'échéance ne portait pas d'heure, la
//! soustraction rendait le décalage New York / UTC — quatre heures de vie
//! accordées à un contrat déjà réglé, sur des échéances qui portaient 82 % du GEX.
//!
//! D'où la règle ici : **l'échéance est un instant, pas une date**, et elle est
//! exprimée en heure de New York tandis que la valorisation l'est en UTC. Les
//! deux types se ressemblent et ne veulent pas dire la même chose, ce qui est
//! précisément la sorte d'erreur qu'on ne voit pas passer — `EcheanceNy` et
//! `InstantReleve` existent pour que le compilateur la refuse.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Offset, TimeZone, Weekday};
use chrono_tz::America::New_York;

/// Jours de bourse dans une année, convention Perfiliev.
pub const JOURS_BOURSE: f64 = 262.0;
/// Jours calendaires, convention CBOE.
pub const JOURS_CALENDAIRES: f64 = 365.0;

/// Plancher du temps restant, en secondes.
///
/// À T strictement nul le gamma diverge. Ce plancher n'est PAS un moyen de
/// traiter les contrats échus — un mort à qui l'on donne une minute de vie a un
/// gamma énorme, et domine la chaîne entière. Les écarter est le travail de
/// [`crate::analyse::filtre_echeances`] ; ceci ne protège que du zéro exact.
const PLANCHER_SECONDES: f64 = 60.0;

/// Une échéance, exprimée en heure de New York.
///
/// Type distinct de [`InstantReleve`] à dessein : les confondre ferait passer le
/// décalage de fuseau pour du temps restant, ce qui est exactement le défaut que
/// ce module a corrigé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EcheanceNy(pub NaiveDateTime);

/// L'instant d'un relevé, en UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct InstantReleve(pub NaiveDateTime);

impl EcheanceNy {
    /// L'échéance ramenée en UTC, seule forme comparable à un relevé.
    ///
    /// Les deux cas tordus du changement d'heure sont tranchés comme le fait le
    /// moteur Python, et pas au hasard : une heure qui n'existe pas (bascule de
    /// printemps) est décalée en avant, une heure vécue deux fois (bascule
    /// d'automne) prend la première occurrence. Laisser l'un des deux échouer
    /// ferait disparaître une échéance deux fois l'an, sans autre symptôme qu'un
    /// GEX qui manque une ligne.
    pub fn en_utc(self) -> NaiveDateTime {
        let mut local = self.0;
        for _ in 0..3 {
            match New_York.from_local_datetime(&local) {
                chrono::offset::LocalResult::Single(t) => {
                    return local - t.offset().fix();
                }
                chrono::offset::LocalResult::Ambiguous(t, _) => {
                    return local - t.offset().fix();
                }
                chrono::offset::LocalResult::None => {
                    // Heure inexistante : on avance jusqu'à en trouver une.
                    local += Duration::hours(1);
                }
            }
        }
        // Trois heures de décalage sans trouver d'heure valide n'existe dans
        // aucune règle de fuseau : mieux vaut le dire que rendre un chiffre.
        panic!("échéance {} introuvable dans le fuseau de New York", self.0)
    }
}

/// Comment mesurer le temps restant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Convention {
    /// Temps réel restant, rapporté à 365 jours. Convention du CBOE.
    Heures,
    /// Jours ouvrés / 262, plancher à un jour. Convention du script de Perfiliev.
    ///
    /// Ce plancher surestime lourdement les 0DTE — un 0DTE à 10 h du matin, c'est
    /// 0,23 jour, pas 1 — et le gamma variant en 1/racine(T), l'écart est massif.
    /// Conservée pour comparer, pas parce qu'elle est juste.
    Bourse,
}

impl Convention {
    /// Le diviseur ramenant une dérivée temporelle à la journée.
    ///
    /// Le charm en a besoin, et doit recevoir CELUI de la convention ayant servi
    /// à calculer T. Les désaccorder donne un charm faux d'un facteur 365/262
    /// sans en changer le signe : rien dans la lecture ne paraîtrait anormal.
    pub fn jours_par_an(self) -> f64 {
        match self {
            Convention::Heures => JOURS_CALENDAIRES,
            Convention::Bourse => JOURS_BOURSE,
        }
    }
}

/// Temps restant en années.
pub fn temps_restant(
    echeance: EcheanceNy,
    releve: InstantReleve,
    convention: Convention,
) -> f64 {
    match convention {
        Convention::Bourse => {
            let jours = jours_ouvres(releve.0.date(), echeance.0.date());
            let jours = if jours == 0 { 1 } else { jours };
            jours as f64 / JOURS_BOURSE
        }
        Convention::Heures => {
            let restant = (echeance.en_utc() - releve.0).num_milliseconds() as f64 / 1000.0;
            restant.max(PLANCHER_SECONDES) / (JOURS_CALENDAIRES * 24.0 * 3600.0)
        }
    }
}

/// Jours ouvrés dans l'intervalle `[debut, fin)`, samedis et dimanches exclus.
///
/// Mêmes bornes que `numpy.busday_count`, dont le moteur Python se sert : la
/// borne de fin est exclue. Les jours fériés ne sont pas retirés — ils ne
/// l'étaient pas non plus côté Python, et les ajouter ici ferait diverger les
/// deux moteurs sur un détail qui ne change pas le régime.
fn jours_ouvres(debut: NaiveDate, fin: NaiveDate) -> i64 {
    if fin < debut {
        return -jours_ouvres(fin, debut);
    }
    let total = (fin - debut).num_days();
    let semaines = total / 7;
    let reste = total % 7;
    let mut compte = semaines * 5;
    let mut jour = debut + Duration::days(semaines * 7);
    for _ in 0..reste {
        if !matches!(jour.weekday(), Weekday::Sat | Weekday::Sun) {
            compte += 1;
        }
        jour += Duration::days(1);
    }
    compte
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echeance(s: &str) -> EcheanceNy {
        EcheanceNy(NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap())
    }
    fn releve(s: &str) -> InstantReleve {
        InstantReleve(NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap())
    }

    /// Figé depuis le moteur Python, sur des cas choisis pour couvrir ce qui casse :
    /// l'été, l'hiver, une échéance du matin, et une bascule d'heure traversée.
    const ORACLE_HEURES: [(&str, &str, f64); 5] = [
        // hebdomadaire PM, heure d'été (EDT = UTC-4)
        ("2026-08-26 16:00:00", "2026-08-25 20:30:16", 2.682_141_045_154_744e-3),
        // mensuelle AM : réglée le matin, six heures et demie plus tôt
        ("2026-09-18 09:30:00", "2026-08-25 20:30:16", 6.495_383_054_287_164e-2),
        // heure d'hiver (EST = UTC-5)
        ("2026-01-16 16:00:00", "2026-01-15 21:00:00", 2.739_726_027_397_26e-3),
        // traverse le passage à l'heure d'été : 71 heures, pas 72
        ("2026-03-09 16:00:00", "2026-03-06 21:00:00", 8.105_022_831_050_229e-3),
        // une minute avant le règlement
        ("2026-08-25 16:00:00", "2026-08-25 19:59:00", 1.902_587_519_025_875e-6),
    ];

    #[test]
    fn convention_heures_egale_le_moteur_python() {
        for (exp, asof, attendu) in ORACLE_HEURES {
            let obtenu = temps_restant(echeance(exp), releve(asof), Convention::Heures);
            let ecart = ((obtenu - attendu) / attendu).abs();
            assert!(
                ecart <= 1e-12,
                "T({exp} vu de {asof}) = {obtenu:e}, Python donne {attendu:e}"
            );
        }
    }

    /// Le cas qui prouve que le fuseau est vraiment traité : du vendredi 6 mars au
    /// lundi 9 mars 2026 il s'écoule 71 heures, pas 72 — l'heure d'été en mange une.
    /// Un calcul en dates nues n'aurait rien vu.
    #[test]
    fn le_passage_a_l_heure_d_ete_retire_une_heure() {
        let t = temps_restant(
            echeance("2026-03-09 16:00:00"),
            releve("2026-03-06 21:00:00"),
            Convention::Heures,
        );
        let heures = t * JOURS_CALENDAIRES * 24.0;
        assert!((heures - 71.0).abs() < 1e-6, "{heures} heures au lieu de 71");
    }

    /// L'été le décalage vaut 4 heures, l'hiver 5. C'est ce décalage-là qui tenait
    /// lieu de temps restant tant que l'échéance était à minuit.
    #[test]
    fn le_decalage_de_new_york_suit_la_saison() {
        assert_eq!(
            echeance("2026-08-26 16:00:00").en_utc(),
            NaiveDateTime::parse_from_str("2026-08-26 20:00:00", "%Y-%m-%d %H:%M:%S").unwrap()
        );
        assert_eq!(
            echeance("2026-01-16 16:00:00").en_utc(),
            NaiveDateTime::parse_from_str("2026-01-16 21:00:00", "%Y-%m-%d %H:%M:%S").unwrap()
        );
    }

    /// Une heure qui n'existe pas — la bascule de printemps saute de 2h à 3h.
    /// Échouer ici ferait disparaître une échéance deux fois l'an.
    #[test]
    fn une_heure_inexistante_est_decalee_en_avant() {
        let saut = echeance("2026-03-08 02:30:00").en_utc();
        let attendu =
            NaiveDateTime::parse_from_str("2026-03-08 07:30:00", "%Y-%m-%d %H:%M:%S").unwrap();
        assert_eq!(saut, attendu, "l'heure inexistante doit avancer d'une heure");
    }

    #[test]
    fn convention_bourse_compte_les_jours_ouvres() {
        for (exp, asof, jours) in [
            ("2026-08-26 16:00:00", "2026-08-25 20:30:16", 1.0),
            ("2026-09-18 09:30:00", "2026-08-25 20:30:16", 18.0),
            ("2026-03-09 16:00:00", "2026-03-06 21:00:00", 1.0),
        ] {
            let t = temps_restant(echeance(exp), releve(asof), Convention::Bourse);
            assert!(
                (t * JOURS_BOURSE - jours).abs() < 1e-9,
                "{exp} : {} jours ouvrés au lieu de {jours}",
                t * JOURS_BOURSE
            );
        }
    }

    /// Le plancher à un jour de la convention Perfiliev, celui qui surestime les
    /// 0DTE. Il est conservé pour rester comparable, pas parce qu'il est juste.
    #[test]
    fn convention_bourse_planche_le_jour_meme_a_un_jour() {
        let t = temps_restant(
            echeance("2026-08-25 16:00:00"),
            releve("2026-08-25 13:00:00"),
            Convention::Bourse,
        );
        assert!((t * JOURS_BOURSE - 1.0).abs() < 1e-9);
    }

    #[test]
    fn jours_ouvres_ignore_les_week_ends() {
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        // vendredi -> lundi : seul le vendredi compte (borne de fin exclue)
        assert_eq!(jours_ouvres(d("2026-03-06"), d("2026-03-09")), 1);
        // une semaine pleine
        assert_eq!(jours_ouvres(d("2026-03-02"), d("2026-03-09")), 5);
        // samedi -> samedi suivant
        assert_eq!(jours_ouvres(d("2026-03-07"), d("2026-03-14")), 5);
        // même jour
        assert_eq!(jours_ouvres(d("2026-03-06"), d("2026-03-06")), 0);
        // à rebours
        assert_eq!(jours_ouvres(d("2026-03-09"), d("2026-03-06")), -1);
    }

    /// Le plancher protège du zéro exact, où le gamma diverge. Il ne rend PAS un
    /// contrat échu utilisable : c'est au filtre d'échéance de l'écarter.
    #[test]
    fn le_plancher_evite_la_division_par_zero() {
        let t = temps_restant(
            echeance("2026-08-25 16:00:00"),
            releve("2026-08-25 20:00:00"),
            Convention::Heures,
        );
        assert!(t > 0.0);
        assert!((t * JOURS_CALENDAIRES * 24.0 * 3600.0 - PLANCHER_SECONDES).abs() < 1e-6);
    }
}
