//! L'écran de séance : un serveur local qui sert ce que le collecteur a écrit.
//!
//! **Il ne calcule rien.** Il lit les trois fichiers du collecteur et les traduit
//! en JSON. Refaire l'analyse à chaque requête dupliquerait `gex-core` dans un
//! second chemin, et deux chemins finissent par diverger — l'écran montrerait
//! alors autre chose que le lecteur.
//!
//! Conception : `docs/superpowers/specs/2026-08-26-ecran-de-seance-design.md`.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use clap::Parser;
use gex_core::analyse::{Analyse, Parametres, analyser, greeks_muets};
use gex_core::contrat::multiplicateur;
use gex_store::series::{Pas, chemin_barres, chemin_niveaux};
use gex_store::{chemin_courant, lire_releve};
use serde::Serialize;

#[derive(Parser, Debug, Clone)]
#[command(name = "gex-web", about = "L'écran de séance, servi en local")]
struct Arguments {
    /// Produit CME : NQ, ES...
    #[arg(default_value = "NQ")]
    produit: String,

    /// Dossier des relevés.
    #[arg(long, default_value = "snapshots")]
    dir: PathBuf,

    /// Port d'écoute.
    #[arg(long, default_value = "8787")]
    port: u16,

    /// Horizon d'échéance du profil, en jours.
    #[arg(long, value_name = "N", default_value = "30")]
    dte_max: i64,

    /// Multiplicateur du contrat. Par défaut celui du produit.
    #[arg(long)]
    contract_size: Option<f64>,
}

/// Une barre, telle que la page l'attend.
#[derive(Serialize)]
struct Barre {
    /// Secondes depuis l'époque : ce que `lightweight-charts` prend.
    time: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
}

/// Un point de la série des niveaux.
#[derive(Serialize)]
struct Niveau {
    time: i64,
    spot: f64,
    zero_gamma: Option<f64>,
    gex: f64,
    charm: f64,
    vanna: f64,
    call_wall: Option<f64>,
    put_wall: Option<f64>,
    /// Les murs vus par l'open interest seul, sans pondération par le gamma.
    ///
    /// Ils répondent à une autre question que les murs gamma : « où y a-t-il le
    /// plus de contrats », et non « où la couverture est-elle la plus sensible ».
    /// Ils divergent souvent, et c'est leur accord qui rend un niveau crédible —
    /// le lecteur en ligne de commande les montre côte à côte pour cette raison.
    call_wall_oi: Option<f64>,
    put_wall_oi: Option<f64>,
    /// Les murs vus par le VOLUME du jour : ou quelqu'un vient de prendre
    /// position, par opposition aux positions accumulees depuis des semaines.
    call_wall_vol: Option<f64>,
    put_wall_vol: Option<f64>,
    /// La volatilité implicite à la monnaie, et la pente du smile.
    ///
    /// Ni l'une ni l'autre n'est un prix : elles vont dans leur propre bande, pas
    /// sur l'axe du sous-jacent.
    iv_atm: Option<f64>,
    skew: Option<f64>,
    /// Le strike où les options de l'échéance la plus proche valent le moins au
    /// règlement.
    ///
    /// Sa dérive porte plus que sa valeur : il ne bouge que quand l'open interest
    /// bouge, donc quand quelqu'un ouvre ou ferme des positions.
    max_pain: Option<f64>,
    /// Les trois greeks de POSITION du book : ce qu'il est, et non le flux qu'il
    /// engendre.
    ///
    /// Ils vont dans leur propre panneau, et jamais avec le GEX, le charm ou le
    /// vanna : un delta dollar se compte en milliards quand un charm se compte en
    /// millions, et le partage d'un axe fabriquerait des croisements qui ne
    /// dépendent que de l'échelle.
    delta: Option<f64>,
    vega: Option<f64>,
    theta: Option<f64>,
    /// L'horizon sous lequel ce point a été calculé.
    ///
    /// L'écran laisse choisir le sien pour l'instant présent ; il a besoin de
    /// celui-ci pour dire sous quel horizon la TRACE, elle, a été tracée. Deux
    /// horizons différents ne sont pas comparables.
    dte_max: Option<f64>,
}

/// Le GEX d'un strike, à l'instant présent.
#[derive(Serialize)]
struct Strike {
    strike: f64,
    gex: f64,
    call_oi: f64,
    put_oi: f64,
}

/// L'état de la collecte, affiché en permanence.
///
/// Un écran qui a l'air vivant alors qu'il est figé est pire qu'un écran vide :
/// le différé est déjà de quinze minutes, et un collecteur arrêté ajouterait un
/// retard invisible par-dessus.
#[derive(Serialize)]
struct Etat {
    /// L'instant du dernier relevé, en secondes depuis l'époque.
    releve: Option<i64>,
    /// Le prix du sous-jacent au dernier relevé.
    spot: Option<f64>,
    /// Nombre de strikes dans le relevé courant.
    strikes: usize,
    /// Le message à afficher quand quelque chose cloche.
    avertissement: Option<String>,
}

/// Ce que le relevé courant vaut **à l'horizon choisi**.
///
/// Ces mêmes grandeurs existent dans la série des niveaux, mais figées à
/// l'horizon du collecteur. Elles sont recalculées ici parce que l'horizon change
/// le chiffre, et pas à la marge : sur un indice, la chaîne entière et le 0–7 DTE
/// donnent des GEX de **signes opposés** pour la même séance.
#[derive(Serialize)]
struct Maintenant {
    gex: f64,
    zero_gamma: Option<f64>,
    call_wall: Option<f64>,
    put_wall: Option<f64>,
    max_pain: Option<f64>,
    /// Les deux strikes où le gamma **net** est le plus concentré.
    ///
    /// Ce ne sont pas les murs : un mur est calculé d'un seul côté — le gamma
    /// call au-dessus du spot, le put en dessous — et contraint par la position
    /// du prix. Ceux-ci prennent le net d'un strike, calls et puts confondus,
    /// sans regarder de quel côté il tombe. Ils coïncident souvent et pas
    /// toujours, et c'est quand ils divergent qu'il y a quelque chose à lire.
    long_gamma: Option<f64>,
    short_gamma: Option<f64>,
    /// Les murs par volume du jour.
    call_wall_vol: Option<f64>,
    put_wall_vol: Option<f64>,
}

/// Ce que l'écran demande : un horizon d'échéance, un pas de chandelier.
#[derive(serde::Deserialize, Debug, Clone, Default)]
struct Demande {
    /// Jours d'échéance au maximum. Absent : celui de la ligne de commande.
    /// `-1` : toutes les échéances.
    dte_max: Option<i64>,
    /// Le pas des chandeliers : `1m`, `15m`, `4h`, `1J`. Absent ou illisible :
    /// la minute, qui est la granularité réellement collectée.
    pas: Option<String>,
}

impl Demande {
    /// Le pas retenu. Un texte illisible retombe sur la minute plutôt que de
    /// refuser la requête : l'écran continuerait de fonctionner, simplement à la
    /// granularité brute — et c'est le pas RETENU qui repart, donc le bouton
    /// allumé sera le bon.
    fn pas(&self) -> Pas {
        self.pas
            .as_deref()
            .and_then(Pas::depuis)
            .unwrap_or(Pas::Minutes(1))
    }
}

/// Le pas écrit comme l'écran l'envoie.
fn nom_pas(pas: Pas) -> String {
    match pas {
        Pas::Seance => "1J".to_string(),
        Pas::Minutes(n) if n % 60 == 0 && n >= 60 => format!("{}h", n / 60),
        Pas::Minutes(n) => format!("{n}m"),
    }
}

/// Le GEX à un niveau de prix hypothétique.
///
/// C'est la réponse à « si le sous-jacent allait là, la couverture amortirait-elle
/// ou amplifierait-elle ? ». Un niveau isolé ne le dit pas ; la courbe entière si,
/// et c'est ce qui permet de peindre le régime derrière le prix.
#[derive(Serialize)]
struct PointProfil {
    niveau: f64,
    gex: f64,
    /// Les dollars que la couverture forcerait à traiter pour aller de ce spot à
    /// ce niveau. Positif : les teneurs de marché devraient acheter.
    ///
    /// Le GEX dit si ça amortit ou amplifie ; celui-ci dit de combien. C'est ce
    /// que les gens appellent le carburant d'un squeeze — et ce n'est toujours
    /// pas une prévision : rien ne dit que le prix ira là.
    carburant: f64,
}

#[derive(Serialize)]
struct Profil {
    etat: Etat,
    strikes: Vec<Strike>,
    /// Le régime en fonction du niveau de prix.
    regime: Vec<PointProfil>,
    /// Le mouvement que le marché price d'ici l'échéance la plus proche.
    attendu: Option<f64>,
    /// **Tous** les niveaux où le régime change de signe, pas seulement celui
    /// qu'on retient.
    ///
    /// Le profil croise zéro plusieurs fois dès que les ailes sont chargées.
    /// N'en montrer qu'un laissait croire à une bascule unique : au-dessus on
    /// amortit, en dessous on amplifie. C'est faux quand il y en a trois, et le
    /// lecteur en ligne de commande le signalait déjà sans les nommer.
    ///
    /// Ce sont les croisements du profil **peint**, donc de la même plage que
    /// lui : une bascule à ±10 % du spot n'est pas dans le champ, et l'écran ne
    /// prétend pas la connaître.
    croisements: Vec<f64>,
    /// Celui des croisements que l'écran retient : le plus proche du spot.
    ///
    /// La ligne orange du graphique vient de la série historique et non d'ici ;
    /// ce champ sert à savoir lequel des croisements est déjà tracé, pour ne pas
    /// le dessiner deux fois.
    zero_gamma: Option<f64>,
    /// Les chiffres du relevé courant, à l'horizon retenu.
    maintenant: Option<Maintenant>,
    /// L'horizon effectivement appliqué, en jours. `None` = toutes les échéances.
    ///
    /// Renvoyé et non déduit : l'écran affiche ce que le serveur a **fait**, pas
    /// ce qu'il a demandé. Un horizon refusé ou corrigé se verrait.
    horizon: Option<i64>,
}

/// Demi-plage du profil de régime, autour du spot.
///
/// Assez large pour couvrir une séance agitée, assez serrée pour que chaque pas
/// vaille une quinzaine de points sur NQ — de quoi peindre un dégradé et non des
/// bandeaux.
const PLAGE_REGIME: f64 = 0.04;
/// Nombre de pas sur cette plage.
const NIVEAUX_REGIME: usize = 160;

fn erreur(message: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::SERVICE_UNAVAILABLE, message.to_string())
}

/// Lequel des horizons présents servir.
///
/// Le plus long par défaut : c'est celui du collecteur, et le plus complet. Un
/// horizon demandé qui n'est pas dans le fichier est **ignoré** plutôt
/// qu'obéi — servir un tableau vide se lirait comme « rien ne cotait », alors
/// que la donnée existe, à un autre horizon.
fn horizon_servi(horizons: &[i64], demande: Option<i64>) -> Option<i64> {
    if horizons.is_empty() {
        return None;
    }
    demande
        .filter(|h| horizons.contains(h))
        .or_else(|| horizons.last().copied())
}

/// L'horizon dit en toutes lettres, pour les messages.
fn nom_horizon(jours: Option<i64>) -> String {
    match jours {
        None => "toutes échéances".to_string(),
        Some(0) => "0 jour (0DTE)".to_string(),
        Some(1) => "1 jour".to_string(),
        Some(n) => format!("{n} jours"),
    }
}

/// Le bandeau d'alerte de l'écran.
///
/// Le lecteur en ligne de commande signale les mêmes choses ; l'écran ne peut pas
/// se contenter de moins sous prétexte qu'il est joli. Un book dont le delta, le
/// vega ou le thêta valent exactement zéro n'est pas un book neutre : c'est une
/// source qui ne les publie pas, et ces trois-là n'ont aucun repli recalculé.
fn avertissement(analyse: &Analyse, cote: bool) -> Option<String> {
    if !cote {
        return Some("Aucun open interest dans le relevé : le marché ne cote pas.".to_string());
    }
    // Le volume des options n'arrive que si la souscription a demandé le tick
    // générique 100. S'il manque, les murs par volume sont simplement absents du
    // graphique — rien ne distinguerait « personne n'a traité » de « la source ne
    // le sert pas ». Un marché ouvert où AUCUN strike n'a traité n'existe pas.
    if analyse.par_strike.iter().all(|s| s.call_vol + s.put_vol == 0.0) {
        return Some(format!(
            "Aucun volume sur les {} strikes du relevé : la source ne sert pas le volume \
             des options. Les murs par volume restent vides — ce n'est pas une séance sans \
             échange, c'est une donnée absente.",
            analyse.par_strike.len(),
        ));
    }

    let muets = greeks_muets(analyse);
    (!muets.is_empty()).then(|| {
        let pluriel = muets.len() > 1;
        format!(
            "{} exactement nul{} sur {} strikes ouverts : la source ne {} publie pas. \
             Ce n'est pas un book neutre, c'est une mesure absente — le delta, le vega et \
             le thêta n'ont aucun repli recalculé, contrairement au gamma.",
            muets.join(", "),
            if pluriel { "s" } else { "" },
            analyse.par_strike.len(),
            if pluriel { "les" } else { "le" },
        )
    })
}

/// La série des chandeliers, à un pas.
#[derive(Serialize)]
struct SerieBarres {
    /// Le pas effectivement appliqué, tel que l'écran l'écrit.
    pas: String,
    /// La minute de la dernière transaction, avant tout regroupement.
    ///
    /// Pas le début du dernier seau : à la séance, il tomberait à 21 h la veille
    /// et l'en-tête annoncerait une transaction vieille de vingt heures. Ce que
    /// l'horloge doit dire, c'est quand le sous-jacent a traité pour la dernière
    /// fois.
    derniere_transaction: Option<i64>,
    points: Vec<Barre>,
}

async fn barres(
    State(args): State<Arc<Arguments>>,
    axum::extract::Query(demande): axum::extract::Query<Demande>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let lues = gex_store::series::lire_barres(&chemin_barres(&args.dir, &args.produit))
        .map_err(erreur)?;
    let derniere_transaction = lues.last().map(|b| b.instant.and_utc().timestamp());

    let pas = demande.pas();
    let points: Vec<Barre> = gex_store::series::agreger_barres(&lues, pas)
        .iter()
        .map(|b| Barre {
            time: b.instant.and_utc().timestamp(),
            open: b.open,
            high: b.high,
            low: b.low,
            close: b.close,
        })
        .collect();
    Ok(axum::Json(SerieBarres {
        pas: nom_pas(pas),
        derniere_transaction,
        points,
    }))
}

/// La série des niveaux, à un horizon.
///
/// Un objet et non un tableau, parce qu'il y a deux questions à poser au fichier
/// et une seule lecture pour y répondre : quels horizons contient-il, et que
/// valent les points de celui qu'on veut.
#[derive(Serialize)]
struct SerieNiveaux {
    /// Les horizons présents dans le fichier, triés. Vide sur un fichier écrit
    /// avant que le collecteur n'enregistre le sien.
    horizons: Vec<i64>,
    /// L'horizon effectivement servi. `None` quand le fichier n'en nomme aucun.
    horizon: Option<i64>,
    /// Le pas appliqué, le même que celui des chandeliers.
    pas: String,
    /// Les points de cet horizon.
    points: Vec<Niveau>,
}

async fn niveaux(
    State(args): State<Arc<Arguments>>,
    axum::extract::Query(demande): axum::extract::Query<Demande>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let lus = gex_store::series::lire_niveaux(&chemin_niveaux(&args.dir, &args.produit))
        .map_err(erreur)?;

    let mut horizons: Vec<i64> = lus.iter().filter_map(|p| p.dte_max.map(|h| h as i64)).collect();
    horizons.sort_unstable();
    horizons.dedup();

    let horizon = horizon_servi(&horizons, demande.dte_max);

    // Un point dont l'horizon est inconnu ne peut être rangé sous AUCUN horizon
    // sans inventer la donnée. Tant que le fichier n'en nomme aucun, ils passent
    // tous — c'est la série d'avant, cohérente avec elle-même. Dès qu'un horizon
    // apparaît, les points muets sont écartés : les mêler à ceux d'un horizon
    // nommé ferait lire deux mesures différentes comme une seule courbe.
    let lus: Vec<_> = match horizon {
        None => lus.to_vec(),
        Some(h) => lus
            .iter()
            .filter(|p| p.dte_max == Some(h as f64))
            .copied()
            .collect(),
    };

    // Le MÊME pas que les chandeliers, par la même fonction : les deux séries
    // doivent tomber sur les mêmes horodatages, sinon la bande du bas ne
    // s'aligne plus sur le prix — et l'alignement est toute sa raison d'être.
    let pas = demande.pas();
    let lus = gex_store::series::agreger_niveaux(&lus, pas);

    let points: Vec<Niveau> = lus
        .iter()
        .map(|p| Niveau {
            time: p.instant.and_utc().timestamp(),
            spot: p.spot,
            zero_gamma: p.zero_gamma,
            gex: p.gex,
            charm: p.charm,
            vanna: p.vanna,
            call_wall: p.call_wall,
            put_wall: p.put_wall,
            call_wall_oi: p.call_wall_oi,
            put_wall_oi: p.put_wall_oi,
            call_wall_vol: p.call_wall_vol,
            put_wall_vol: p.put_wall_vol,
            iv_atm: p.iv_atm,
            skew: p.skew,
            max_pain: p.max_pain,
            delta: p.delta,
            vega: p.vega,
            theta: p.theta,
            dte_max: p.dte_max,
        })
        .collect();
    Ok(axum::Json(SerieNiveaux { horizons, horizon, pas: nom_pas(pas), points }))
}

/// Le profil par strike de l'instant présent.
///
/// Il vient du relevé courant et ne concerne que maintenant : un profil passé
/// n'aurait aucun sens, le book ayant changé. C'est aussi pourquoi la série des
/// niveaux ne le porte pas.
async fn profil(
    State(args): State<Arc<Arguments>>,
    axum::extract::Query(demande): axum::extract::Query<Demande>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // L'horizon vient de l'écran, mais le serveur décide. `-1` veut dire toutes
    // les échéances ; une valeur absurde est ramenée dans les bornes plutôt que
    // refusée — et c'est l'horizon RETENU qui repart, pas celui demandé.
    let horizon: Option<i64> = match demande.dte_max {
        None => Some(args.dte_max),
        Some(n) if n < 0 => None,
        Some(n) => Some(n.min(3650)),
    };

    let source = chemin_courant(&args.dir, &args.produit);
    if !source.exists() {
        return Ok(axum::Json(Profil {
            etat: Etat {
                releve: None,
                spot: None,
                strikes: 0,
                avertissement: Some(format!(
                    "Aucun relevé en {}. Lance : gex-collector {}",
                    source.display(),
                    args.produit
                )),
            },
            strikes: Vec::new(),
            regime: Vec::new(),
            attendu: None,
            croisements: Vec::new(),
            zero_gamma: None,
            maintenant: None,
            horizon,
        }));
    }

    let chaine = lire_releve(&source).map_err(erreur)?;
    let taille = match args.contract_size {
        Some(t) => t,
        None => multiplicateur(&args.produit).map_err(erreur)?,
    };
    let analyse = analyser(
        &chaine,
        &Parametres {
            taille_contrat: taille,
            dte_max: horizon,
            // Le profil sert ici de FOND derrière le prix, pas de tableau : il lui
            // faut du grain là où le prix se trouve, pas une couverture large et
            // grossière. Les défauts du lecteur — ±20 % en soixante pas, soit près
            // de deux cents points par pas — donneraient une seule bande à l'écran.
            plage: PLAGE_REGIME,
            niveaux: NIVEAUX_REGIME,
            ..Default::default()
        },
    );

    // Un horizon vide n'est pas une panne. Hors séance, « 0DTE » ne contient rien
    // — l'échéance du jour est déjà réglée — et c'est une réponse, pas une
    // erreur. La rendre en 503 faisait afficher « le serveur ne répond pas » sur
    // tout l'écran pour un clic parfaitement légitime.
    let analyse = match analyse {
        Ok(a) => a,
        Err(e) => {
            return Ok(axum::Json(Profil {
                etat: Etat {
                    releve: Some(chaine.releve.0.and_utc().timestamp()),
                    spot: Some(chaine.spot),
                    strikes: 0,
                    avertissement: Some(format!(
                        "Aucune échéance dans l'horizon {} : {e}",
                        nom_horizon(horizon)
                    )),
                },
                strikes: Vec::new(),
                regime: Vec::new(),
                attendu: None,
                croisements: Vec::new(),
                zero_gamma: None,
                maintenant: None,
                horizon,
            }));
        }
    };

    let strikes: Vec<Strike> = analyse
        .par_strike
        .iter()
        .map(|s| Strike {
            strike: s.strike,
            gex: s.gex,
            call_oi: s.call_oi,
            put_oi: s.put_oi,
        })
        .collect();

    // Un relevé sans open interest n'a pas un GEX nul : il n'en a pas. Le dire
    // évite de lire une ligne plate comme une mesure.
    let cote = analyse
        .lignes
        .iter()
        .any(|l| l.ligne.call.open_interest + l.ligne.put.open_interest > 0.0);

    Ok(axum::Json(Profil {
        etat: Etat {
            releve: Some(chaine.releve.0.and_utc().timestamp()),
            spot: Some(analyse.spot),
            strikes: strikes.len(),
            avertissement: avertissement(&analyse, cote),
        },
        strikes,
        regime: analyse
            .niveaux
            .iter()
            .zip(analyse.profil.iter())
            .zip(analyse.carburant.iter())
            .map(|((niveau, gex), carburant)| PointProfil {
                niveau: *niveau,
                gex: *gex,
                carburant: *carburant,
            })
            .collect(),
        attendu: analyse.attendu,
        croisements: analyse.croisements,
        zero_gamma: analyse.zero_gamma,
        maintenant: Some(Maintenant {
            gex: analyse.gex,
            zero_gamma: analyse.zero_gamma,
            call_wall: analyse.murs.call,
            put_wall: analyse.murs.put,
            max_pain: analyse.max_pain,
            long_gamma: analyse.gamma_majeur.long,
            short_gamma: analyse.gamma_majeur.court,
            call_wall_vol: analyse.murs.call_vol,
            put_wall_vol: analyse.murs.put_vol,
        }),
        horizon,
    }))
}

async fn page() -> impl IntoResponse {
    Html(include_str!("../static/index.html"))
}

/// La bibliothèque de graphiques, servie depuis le disque.
///
/// Pas depuis un réseau de diffusion : l'écran doit fonctionner sans réseau,
/// comme le reste du dépôt.
async fn graphiques() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("../static/lightweight-charts.js"),
    )
}

async fn style() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css")],
        include_str!("../static/style.css"),
    )
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Arc::new(Arguments::parse());
    let port = args.port;

    let routes = Router::new()
        .route("/", get(page))
        .route("/lightweight-charts.js", get(graphiques))
        .route("/style.css", get(style))
        .route("/api/barres", get(barres))
        .route("/api/niveaux", get(niveaux))
        .route("/api/profil", get(profil))
        .with_state(args);

    // Uniquement en local : ces relevés sont à toi, et rien ne justifie de les
    // exposer au réseau.
    let adresse = format!("127.0.0.1:{port}");
    let ecoute = match tokio::net::TcpListener::bind(&adresse).await {
        Ok(e) => e,
        Err(err) => {
            eprintln!("Erreur : impossible d'écouter sur {adresse} — {err}");
            return ExitCode::FAILURE;
        }
    };

    println!("L'écran de séance est sur http://{adresse}");
    println!("Ctrl+C pour arrêter.");
    if let Err(err) = axum::serve(ecoute, routes).await {
        eprintln!("Erreur : {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le plus long par défaut : c'est celui du collecteur, et le plus complet.
    #[test]
    fn sans_demande_le_plus_long_est_servi() {
        assert_eq!(horizon_servi(&[0, 1, 7, 30], None), Some(30));
        assert_eq!(horizon_servi(&[0], None), Some(0));
    }

    /// Un horizon demandé et présent est servi tel quel.
    #[test]
    fn un_horizon_present_est_servi() {
        assert_eq!(horizon_servi(&[0, 1, 7, 30], Some(1)), Some(1));
        assert_eq!(horizon_servi(&[0, 1, 7, 30], Some(0)), Some(0));
    }

    /// Un horizon absent est ignoré, pas obéi : servir un tableau vide se lirait
    /// « rien ne cotait » alors que la donnée existe, à un autre horizon.
    #[test]
    fn un_horizon_absent_retombe_sur_le_plus_long() {
        assert_eq!(horizon_servi(&[0, 1, 7, 30], Some(3)), Some(30));
        assert_eq!(horizon_servi(&[0, 1, 7, 30], Some(-1)), Some(30));
    }

    /// Un fichier écrit avant que le collecteur n'enregistre l'horizon n'en nomme
    /// aucun. Rien n'est servi comme horizon, et l'écran le dit.
    #[test]
    fn un_fichier_sans_horizon_n_en_nomme_aucun() {
        assert_eq!(horizon_servi(&[], None), None);
        assert_eq!(horizon_servi(&[], Some(7)), None);
    }

    /// L'horizon en toutes lettres, pour les messages de l'écran.
    #[test]
    fn l_horizon_se_dit_en_toutes_lettres() {
        assert_eq!(nom_horizon(Some(0)), "0 jour (0DTE)");
        assert_eq!(nom_horizon(Some(1)), "1 jour");
        assert_eq!(nom_horizon(Some(30)), "30 jours");
        assert_eq!(nom_horizon(None), "toutes échéances");
    }
}
