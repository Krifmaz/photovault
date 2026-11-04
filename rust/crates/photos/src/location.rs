use std::path::Path;

use ente_assets::download::CancellationToken;
use ente_assets::{Asset, AssetFile, AssetStore};
use ente_location::{
    CityIndex, CityMatch, Coordinate, CountryCode, CountryIndex, CountryView, UrbanCenterIndex,
};

// PHOTOVAULT: strip - Ente's CDN, kept only as the default of `open` for
// upstream callers outside the Photovault Photos app (the CLI). The app uses
// `open_from` with its own server's `/static-assets/location/v2`.
const ASSET_URL: &str = "https://assets.ente.com/location/v2";
const CITY_FILE: &str = "cities.bin";
const URBAN_CENTER_FILE: &str = "urban-centres.bin";
const COUNTRY_FILE: &str = "countries.bin";
const DISPUTE_FILE: &str = "disputes.bin";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Asset(#[from] ente_assets::download::Error),
    #[error(transparent)]
    Location(#[from] ente_location::Error),
    #[error(transparent)]
    CountryCode(#[from] ente_location::InvalidCountryCode),
}

pub struct LocationIndex {
    cities: CityIndex,
    urban_centers: UrbanCenterIndex,
    countries: CountryIndex,
}

impl LocationIndex {
    pub async fn open(asset_root: impl AsRef<Path>) -> Result<Self, Error> {
        Self::open_from(asset_root, ASSET_URL).await
    }

    /// PHOTOVAULT: strip - like `open`, but downloads missing files from
    /// `base_url` (a directory with Ente's `location/v2` layout; the files
    /// are still checked against the pinned sizes and SHA-256 sums below).
    pub async fn open_from(asset_root: impl AsRef<Path>, base_url: &str) -> Result<Self, Error> {
        let asset = asset(base_url)?;
        let store = AssetStore::new(asset_root.as_ref());
        let directory = store.asset_dir(&asset);
        store
            .download(&[asset], |_| {}, CancellationToken::new())
            .await?;
        Ok(Self {
            cities: CityIndex::from_path(directory.join(CITY_FILE))?,
            urban_centers: UrbanCenterIndex::from_path(directory.join(URBAN_CENTER_FILE))?,
            countries: CountryIndex::from_paths(
                directory.join(COUNTRY_FILE),
                directory.join(DISPUTE_FILE),
            )?,
        })
    }

    pub fn is_downloaded(asset_root: impl AsRef<Path>) -> bool {
        asset(ASSET_URL)
            .is_ok_and(|asset| AssetStore::new(asset_root.as_ref()).is_downloaded(&asset))
    }

    pub fn cities(&self) -> Vec<ente_location::City> {
        self.cities.search("", usize::MAX)
    }

    pub fn group_cities(&self, coordinates: &[Coordinate], query: &str) -> Vec<CityMatch> {
        self.urban_centers
            .match_coordinates_with_cities(&self.cities, coordinates, query)
    }

    pub fn group_countries(
        &self,
        coordinates: &[Coordinate],
        region: Option<&str>,
    ) -> Result<Vec<ente_location::CountryGroup>, Error> {
        let view = match region {
            Some(region) => CountryView::Region(region.parse()?),
            None => CountryView::SourceDefault,
        };
        Ok(self.countries.group(coordinates, view)?.countries)
    }

    pub fn country_names(&self) -> impl Iterator<Item = (CountryCode, &str)> {
        self.cities.country_names()
    }

    pub fn country_name(&self, code: CountryCode) -> Option<&str> {
        self.cities.country_name(code)
    }
}

fn asset(base_url: &str) -> Result<Asset, ente_assets::download::Error> {
    Asset::files(
        vec!["location".into(), "v2".into()],
        vec![
            file(
                base_url,
                CITY_FILE,
                2_809_615,
                "898dca892a71fd601ae8e75e5c55fd6d4591e4c98de335d9b33e7f076aa668f5",
            ),
            file(
                base_url,
                URBAN_CENTER_FILE,
                1_825_064,
                "e981b0383e6502ede56b52d5be96cda66f53d7352492e677d8978467c106dab0",
            ),
            file(
                base_url,
                COUNTRY_FILE,
                2_504_017,
                "3ead1ef2bb03b4ffa95813df4316cf149d37c82d16a322352bd6e3b19218c796",
            ),
            file(
                base_url,
                DISPUTE_FILE,
                61_543,
                "c44d0af6ad40a3b195bc10cb87512c16998a20e837af190ff1a46b00170dfd7b",
            ),
        ],
    )
}

fn file(base_url: &str, name: &str, size: u64, sha256: &str) -> AssetFile {
    AssetFile {
        name: name.into(),
        url: format!("{}/{name}", base_url.trim_end_matches('/')),
        size,
        sha256: sha256.into(),
    }
}
