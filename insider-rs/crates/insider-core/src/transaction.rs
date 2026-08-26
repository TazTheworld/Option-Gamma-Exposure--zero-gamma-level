//! Ce qu'une déclaration décrit : qui a échangé quoi, quand, et pour combien.
//!
//! Les trois sources parlent de la même chose dans trois formats différents. Le
//! dire avec les mêmes types est ce qui permet de les concaténer sans retouche —
//! et ce qui empêche de comparer un achat d'actions à un exercice d'options en
//! croyant comparer deux achats.

use chrono::NaiveDate;

use crate::montant::Montant;

/// D'où vient la déclaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// Formulaire 4 de la SEC : dirigeants, administrateurs, détenteurs de plus
    /// de 10 % d'une société cotée.
    Entreprise,
    /// Chambre des représentants, sous le STOCK Act.
    Chambre,
    /// Sénat, sous le STOCK Act.
    Senat,
}

impl Source {
    /// Le libellé affiché.
    pub fn nom(&self) -> &'static str {
        match self {
            Source::Entreprise => "entreprise",
            Source::Chambre => "chambre",
            Source::Senat => "senat",
        }
    }

    /// Le délai réglementaire de déclaration, en jours.
    ///
    /// Deux jours ouvrés pour un dirigeant, quarante-cinq pour un élu. C'est ce
    /// qui rend les deux sources incomparables en fraîcheur : une déclaration
    /// d'élu décrit un mois et demi de passé.
    pub fn delai_reglementaire(&self) -> u32 {
        match self {
            Source::Entreprise => 2,
            Source::Chambre | Source::Senat => 45,
        }
    }
}

/// Qui détient réellement le titre.
///
/// L'absence de mention signifie le déclarant lui-même, ce qui n'est **pas** la
/// même chose qu'une donnée manquante — d'où une variante pour chacun.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detenteur {
    /// Le déclarant, en son nom propre.
    Declarant,
    /// Le conjoint.
    Conjoint,
    /// Un enfant à charge.
    Enfant,
    /// Un compte joint.
    Joint,
    /// Détenu indirectement — trust, société, plan de retraite. Le libellé dit
    /// souvent par quel intermédiaire, et c'est parfois la seule chose qui
    /// distingue deux lignes autrement identiques.
    Indirect(&'static str),
    /// La source n'a rien dit.
    Inconnu,
}

impl Detenteur {
    /// Le libellé affiché.
    pub fn nom(&self) -> &str {
        match self {
            Detenteur::Declarant => "le déclarant",
            Detenteur::Conjoint => "conjoint",
            Detenteur::Enfant => "enfant à charge",
            Detenteur::Joint => "compte joint",
            Detenteur::Indirect(par) => par,
            Detenteur::Inconnu => "non précisé",
        }
    }
}

/// Ce que l'opération fait à la position.
///
/// Le signe rend les mouvements cumulables. Un échange ne fait ni acquérir ni
/// céder : lui donner un signe le ferait entrer dans des totaux où il n'a rien à
/// faire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sens {
    /// La position augmente.
    Acquisition,
    /// La position diminue.
    Cession,
    /// Ni l'un ni l'autre : échange, conversion, détention déclarée.
    Neutre,
}

impl Sens {
    /// +1, -1 ou 0.
    pub fn signe(&self) -> i8 {
        match self {
            Sens::Acquisition => 1,
            Sens::Cession => -1,
            Sens::Neutre => 0,
        }
    }
}

/// Ce que l'initié a fait, et si c'est une décision.
///
/// **La distinction porte tout le module.** La majorité des déclarations ne sont
/// pas des décisions d'investissement : une attribution est une rémunération, un
/// exercice suivi d'une retenue fiscale est mécanique, une vente est souvent
/// programmée des mois à l'avance. Les mélanger ferait passer un plan de
/// rémunération pour un signal de conviction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nature {
    /// L'initié engage son argent à un moment qu'il choisit.
    Decision,
    /// Il reçoit des titres au titre de sa rémunération.
    Remuneration,
    /// Le mouvement découle d'un autre : exercice, conversion, retenue fiscale.
    Mecanique,
    /// Une position déclarée sans mouvement.
    Detention,
    /// Le reste : donation, succession, transfert.
    Autre,
}

impl Nature {
    /// Le libellé affiché.
    pub fn nom(&self) -> &'static str {
        match self {
            Nature::Decision => "décision",
            Nature::Remuneration => "rémunération",
            Nature::Mecanique => "mécanique",
            Nature::Detention => "détention",
            Nature::Autre => "autre",
        }
    }
}

/// Une opération déclarée.
///
/// Les champs facultatifs le sont parce qu'une source ne les sert pas, jamais
/// parce qu'on ne les a pas lus. `date_notification` est vide chez le Sénat, qui
/// ne la publie pas ; la recopier depuis `date` inventerait un délai nul.
#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    /// D'où vient la déclaration.
    pub source: Source,
    /// Le nom du déclarant.
    pub declarant: String,
    /// Sa fonction, telle que déclarée.
    pub role: Option<String>,
    /// Le jour de l'opération.
    pub date: Option<NaiveDate>,
    /// Le jour où le déclarant dit en avoir été informé.
    pub date_notification: Option<NaiveDate>,
    /// Le symbole boursier, absent sur les obligations et les fonds non cotés.
    pub symbole: Option<String>,
    /// Le libellé du titre.
    pub actif: Option<String>,
    /// Ce que l'opération fait à la position.
    pub sens: Sens,
    /// Si c'est une décision, et sinon quoi.
    pub nature: Nature,
    /// Qui détient réellement.
    pub detenteur: Detenteur,
    /// Le montant, tel que déclaré.
    pub montant: Montant,
    /// L'adresse du document d'origine.
    pub document: Option<String>,
}

impl Operation {
    /// Le délai entre l'opération et sa notification, en jours.
    ///
    /// Une information en soi : un compte géré par un tiers notifie tard, un
    /// ordre passé en propre notifie le jour même. `None` quand la source ne
    /// publie pas de date de notification — pas zéro, qui se lirait comme
    /// « notifié immédiatement ».
    pub fn delai_notification(&self) -> Option<i64> {
        let (date, notifiee) = (self.date?, self.date_notification?);
        Some((notifiee - date).num_days())
    }

    /// L'opération engage-t-elle un choix de l'initié ?
    pub fn est_une_decision(&self) -> bool {
        self.nature == Nature::Decision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jour(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn operation() -> Operation {
        Operation {
            source: Source::Chambre,
            declarant: "Hon. Jane Doe".to_string(),
            role: Some("Member".to_string()),
            date: Some(jour("2026-03-16")),
            date_notification: Some(jour("2026-03-20")),
            symbole: Some("AMZN".to_string()),
            actif: Some("Amazon.com, Inc.".to_string()),
            sens: Sens::Acquisition,
            nature: Nature::Decision,
            detenteur: Detenteur::Declarant,
            montant: Montant::Fourchette {
                bas: 1_001.0,
                haut: 15_000.0,
            },
            document: None,
        }
    }

    #[test]
    fn le_delai_se_compte_en_jours() {
        assert_eq!(operation().delai_notification(), Some(4));
    }

    /// Zéro se lirait comme « notifié immédiatement » : le Sénat ne publie pas
    /// cette date, et l'inventer effacerait la distinction.
    #[test]
    fn sans_date_de_notification_il_n_y_a_pas_de_delai() {
        let mut o = operation();
        o.date_notification = None;
        assert_eq!(o.delai_notification(), None);
    }

    /// Deux jours ouvrés contre quarante-cinq : les deux sources ne sont pas
    /// comparables en fraîcheur.
    #[test]
    fn les_delais_reglementaires_different_selon_la_source() {
        assert_eq!(Source::Entreprise.delai_reglementaire(), 2);
        assert_eq!(Source::Chambre.delai_reglementaire(), 45);
        assert_eq!(Source::Senat.delai_reglementaire(), 45);
    }

    /// Un échange ne fait ni acquérir ni céder : lui donner un signe le ferait
    /// entrer dans des totaux où il n'a rien à faire.
    #[test]
    fn un_echange_ne_pese_dans_aucun_total() {
        assert_eq!(Sens::Neutre.signe(), 0);
        assert_eq!(Sens::Acquisition.signe(), 1);
        assert_eq!(Sens::Cession.signe(), -1);
    }

    /// La distinction qui porte tout le module : une attribution n'est pas un
    /// achat, même si les deux augmentent la position.
    #[test]
    fn seule_une_decision_engage_l_initie() {
        let mut o = operation();
        assert!(o.est_une_decision());
        o.nature = Nature::Remuneration;
        assert!(!o.est_une_decision());
        // Et elle reste une acquisition : les deux axes sont indépendants.
        assert_eq!(o.sens, Sens::Acquisition);
    }

    #[test]
    fn l_intermediaire_d_une_detention_indirecte_est_nomme() {
        assert_eq!(Detenteur::Indirect("par un trust familial").nom(),
                   "par un trust familial");
        assert_eq!(Detenteur::Declarant.nom(), "le déclarant");
        // « non précisé » n'est pas « le déclarant » : la source n'a rien dit.
        assert_eq!(Detenteur::Inconnu.nom(), "non précisé");
    }
}
