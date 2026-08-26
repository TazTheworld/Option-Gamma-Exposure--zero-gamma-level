//! Les chandeliers du sous-jacent.
//!
//! Le dépôt n'a jamais eu de série de prix : il calculait des niveaux sans rien
//! sur quoi les poser. IB sert les barres, donc il n'y a pas de source à chercher
//! — seulement deux appels à câbler.
//!
//! **`stream()` plutôt que `realtime_bars`.** Les barres temps réel d'IB sont
//! figées à cinq secondes et ne s'agrègent pas côté serveur : en faire des barres
//! d'une minute demanderait de les recoller nous-mêmes, avec la question des trous
//! quand la connexion coupe. `stream()` rend directement la granularité demandée
//! et **renvoie l'historique d'abord**, ce qui supprime la couture entre
//! l'amorçage et le suivi.

use chrono::{DateTime, NaiveDateTime};
use gex_store::series::Barre;
use ibapi::contracts::Contract;
use ibapi::market_data::TradingHours;
use ibapi::market_data::historical::{BarSize, BarTimestamp, Duration, WhatToShow};

use crate::client::{ErreurIb, Passerelle};

/// Profondeur d'historique demandée à l'amorçage, en jours.
///
/// Deux, et pas trente : la série sur disque garde l'historique long, IB ne sert
/// qu'à combler ce qui manque depuis le dernier arrêt. Demander un mois à chaque
/// démarrage ferait payer une longue requête pour des barres qu'on a déjà.
pub const PROFONDEUR_JOURS: i32 = 2;

/// Traduit l'horodatage d'une barre en instant UTC.
///
/// Ce dépôt s'est fait prendre trois fois par une heure qui se dérobe. `ibapi`
/// rend un instant absolu pour les barres intraday et une simple date pour les
/// barres journalières — cette dernière n'a pas d'heure, donc pas de place ici.
fn en_utc(horodatage: BarTimestamp) -> Option<NaiveDateTime> {
    match horodatage {
        BarTimestamp::DateTime(t) => {
            DateTime::from_timestamp(t.unix_timestamp(), 0).map(|d| d.naive_utc())
        }
        // Une barre journalière n'a pas d'heure : la placer à minuit inventerait un
        // instant, et l'axe de temps de l'écran est à la minute.
        BarTimestamp::Date(_) => None,
    }
}

/// Une barre d'IB vers une barre du dépôt.
fn traduire(b: &ibapi::market_data::historical::Bar) -> Option<Barre> {
    Some(Barre {
        instant: en_utc(b.date)?,
        open: b.open,
        high: b.high,
        low: b.low,
        close: b.close,
        volume: b.volume,
    })
}

impl Passerelle {
    /// Les barres d'une minute du contrat, sur les derniers jours.
    ///
    /// `TradingHours::Extended` et non `Regular` : NQ se traite près de
    /// vingt-quatre heures sur vingt-quatre, et le gamma ne cesse pas d'exister la
    /// nuit. Se limiter à la séance régulière ferait un trou dans le graphique là
    /// où il se passe quelque chose.
    ///
    /// `WhatToShow::Trades` : ce sont les transactions qui font les chandeliers.
    /// `MidPoint` donnerait une série plus lisse et moins vraie.
    pub fn barres(&self, contrat: &Contract, jours: i32) -> Result<Vec<Barre>, ErreurIb> {
        let historique = self
            .client()
            .historical_data(contrat, BarSize::Min)
            .what_to_show(WhatToShow::Trades)
            .trading_hours(TradingHours::Extended)
            .duration(Duration::days(jours.max(1)))
            .fetch()
            .map_err(|e| ErreurIb::Requete(e.to_string()))?;

        // Une barre sans instant exploitable est écartée plutôt que placée
        // arbitrairement : un chandelier au mauvais moment est pire qu'un trou.
        Ok(historique.bars.iter().filter_map(traduire).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

    #[test]
    fn une_barre_intraday_devient_un_instant_utc() {
        // 26 août 2026, 09:15:00 UTC
        let brut = OffsetDateTime::from_unix_timestamp(1_787_735_700).unwrap();
        let instant = en_utc(BarTimestamp::DateTime(brut)).unwrap();
        assert_eq!(
            instant.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-08-26 09:15:00"
        );
    }

    /// Une barre journalière n'a pas d'heure : la placer à minuit inventerait un
    /// instant, et l'axe de temps de l'écran est à la minute.
    #[test]
    fn une_barre_journaliere_n_a_pas_sa_place() {
        let jour = time::Date::from_calendar_date(2026, time::Month::August, 26).unwrap();
        assert_eq!(en_utc(BarTimestamp::Date(jour)), None);
    }
}
