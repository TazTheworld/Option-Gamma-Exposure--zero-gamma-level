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
use gex_core::analyse::{Parametres, analyser};
use gex_core::contrat::multiplicateur;
use gex_store::series::{chemin_barres, chemin_niveaux};
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

#[derive(Serialize)]
struct Profil {
    etat: Etat,
    strikes: Vec<Strike>,
}

fn erreur(message: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::SERVICE_UNAVAILABLE, message.to_string())
}

async fn barres(State(args): State<Arc<Arguments>>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let lues = gex_store::series::lire_barres(&chemin_barres(&args.dir, &args.produit))
        .map_err(erreur)?;
    let sortie: Vec<Barre> = lues
        .iter()
        .map(|b| Barre {
            time: b.instant.and_utc().timestamp(),
            open: b.open,
            high: b.high,
            low: b.low,
            close: b.close,
        })
        .collect();
    Ok(axum::Json(sortie))
}

async fn niveaux(State(args): State<Arc<Arguments>>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let lus = gex_store::series::lire_niveaux(&chemin_niveaux(&args.dir, &args.produit))
        .map_err(erreur)?;
    let sortie: Vec<Niveau> = lus
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
        })
        .collect();
    Ok(axum::Json(sortie))
}

/// Le profil par strike de l'instant présent.
///
/// Il vient du relevé courant et ne concerne que maintenant : un profil passé
/// n'aurait aucun sens, le book ayant changé. C'est aussi pourquoi la série des
/// niveaux ne le porte pas.
async fn profil(State(args): State<Arc<Arguments>>) -> Result<impl IntoResponse, (StatusCode, String)> {
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
            dte_max: Some(args.dte_max),
            ..Default::default()
        },
    )
    .map_err(erreur)?;

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
            avertissement: (!cote).then(|| {
                "Aucun open interest dans le relevé : le marché ne cote pas.".to_string()
            }),
        },
        strikes,
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
