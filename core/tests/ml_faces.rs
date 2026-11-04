//! Faces and people (contract v1.2): Ente's clustering on synthetic
//! embeddings, the `cgroup` JSON, and the people flows end to end against
//! the real Photovault server in process (user entities encrypted exactly
//! as the Ente apps read them).

mod support;

use std::io::Read as _;

use photovault_core::api::Api;
use photovault_core::crypto;
use photovault_core::ml::faces::cgroup::{CGroupData, FaceCluster};
use photovault_core::ml::faces::cluster::{
    self, ClusterFace, RemoteGroup, cluster_faces, is_bad_face, is_clusterable,
};
use photovault_core::ml::{FileFace, Person};
use reqwest::Method;
use serde_json::{Value, json};
use support::{TestCore, jpeg, server, signup};

// ---------------------------------------------------------------- vectors

/// xorshift64*: deterministic noise.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d);
        (v >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
    }

    fn unit(&mut self) -> Vec<f32> {
        unit((0..192).map(|_| self.next()).collect())
    }
}

fn unit(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= n);
    v
}

/// `Σ coef·vec + noise·random`, normalised.
fn mix(rng: &mut Rng, parts: &[(f32, &[f32])], noise: f32) -> Vec<f32> {
    let r = rng.unit();
    unit(
        (0..192)
            .map(|i| parts.iter().map(|(c, v)| c * v[i]).sum::<f32>() + noise * r[i])
            .collect(),
    )
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

// ------------------------------------------------------- pure clustering

fn run(
    faces: &[(String, Vec<f32>, bool)],
    groups: &[RemoteGroup],
    local: Vec<FaceCluster>,
) -> cluster::Clustering {
    let cf: Vec<ClusterFace> = faces
        .iter()
        .map(|(id, _, bad)| ClusterFace {
            face_id: id.clone(),
            is_bad: *bad,
        })
        .collect();
    let mut n = 0;
    cluster_faces(
        &cf,
        groups,
        local,
        |idx| idx.iter().map(|&i| Some(faces[i].1.clone())).collect(),
        || {
            n += 1;
            format!("cluster_{n}")
        },
    )
}

fn cluster_of<'a>(c: &'a cluster::Clustering, face: &str) -> &'a str {
    &c.clusters
        .iter()
        .find(|c| c.faces.iter().any(|f| f == face))
        .expect("face is clustered")
        .id
}

#[test]
fn two_people_separate_and_the_same_person_merges() {
    let mut rng = Rng(7);
    let (a, b) = (rng.unit(), rng.unit());
    let mut faces = Vec::new();
    for i in 0..12 {
        let (base, who) = if i % 2 == 0 { (&a, "a") } else { (&b, "b") };
        faces.push((
            format!("{i}_{who}"),
            mix(&mut rng, &[(1.0, base)], 0.3),
            false,
        ));
    }
    // Same-person faces are well above 0.76, different people near 0.
    assert!(dot(&faces[0].1, &faces[2].1) > 0.8);
    assert!(dot(&faces[0].1, &faces[1].1).abs() < 0.4);
    let c = run(&faces, &[], Vec::new());
    assert_eq!(c.clusters.len(), 2, "{:?}", c.clusters);
    let ca = cluster_of(&c, "0_a");
    let cb = cluster_of(&c, "1_b");
    assert_ne!(ca, cb);
    for (id, _, _) in &faces {
        let want = if id.ends_with('a') { ca } else { cb };
        assert_eq!(cluster_of(&c, id), want);
    }
    assert_eq!(c.modified.len(), 2);

    // Incremental: nothing new, nothing modified, nothing loaded.
    let again = cluster_faces(
        &faces
            .iter()
            .map(|(id, _, bad)| ClusterFace {
                face_id: id.clone(),
                is_bad: *bad,
            })
            .collect::<Vec<_>>(),
        &[],
        c.clusters.clone(),
        |_| panic!("no embeddings are needed when every face is clustered"),
        || unreachable!(),
    );
    assert!(again.modified.is_empty());
    assert_eq!(again.clusters, c.clusters);
}

#[test]
fn bad_faces_need_a_closer_match_and_low_quality_faces_are_filtered() {
    // Ente's filters.
    assert!(!is_clusterable(0.95, 9.0), "blurry");
    assert!(!is_clusterable(0.79, 300.0), "low score");
    assert!(is_clusterable(0.95, 300.0));
    let straight = [(0.3, 0.3), (0.7, 0.3), (0.5, 0.5), (0.35, 0.7), (0.65, 0.7)];
    assert!(is_bad_face(0.95, 30.0, &straight));
    assert!(!is_bad_face(0.95, 300.0, &straight));

    // A face 0.80 similar joins a good face (0.76) but not a bad one (0.84).
    let mut rng = Rng(11);
    let base = rng.unit();
    let other = rng.unit();
    let other = unit(
        other
            .iter()
            .zip(&base)
            .map(|(o, b)| o - dot(&other, &base) * b)
            .collect(),
    );
    let close = unit(
        base.iter()
            .zip(&other)
            .map(|(b, o)| 0.8 * b + 0.6 * o)
            .collect(),
    );
    assert!((dot(&base, &close) - 0.8).abs() < 1e-3);
    let good = run(
        &[
            ("1_x".into(), base.clone(), false),
            ("2_y".into(), close.clone(), false),
        ],
        &[],
        Vec::new(),
    );
    assert_eq!(good.clusters.len(), 1);
    let bad = run(
        &[("1_x".into(), base, true), ("2_y".into(), close, false)],
        &[],
        Vec::new(),
    );
    assert_eq!(bad.clusters.len(), 2);
}

#[test]
fn remote_clusters_grow_and_rejected_faces_stay_out() {
    let mut rng = Rng(3);
    let a = rng.unit();
    let faces: Vec<(String, Vec<f32>, bool)> = (0..6)
        .map(|i| (format!("{i}_a"), mix(&mut rng, &[(1.0, &a)], 0.3), false))
        .collect();
    let group = RemoteGroup {
        updated_at: 5,
        assigned: vec![FaceCluster {
            id: "cluster_remote".into(),
            faces: vec!["0_a".into(), "1_a".into(), "5_a".into()],
        }],
        rejected_face_ids: vec!["5_a".into()],
    };
    let c = run(&faces, &[group], Vec::new());
    // New faces of the person join the remote cluster…
    assert!(c.modified.contains("cluster_remote"));
    for f in ["0_a", "1_a", "2_a", "3_a", "4_a"] {
        assert_eq!(cluster_of(&c, f), "cluster_remote");
    }
    // …but the rejected face never goes back to it.
    assert_ne!(cluster_of(&c, "5_a"), "cluster_remote");
    // The remote cluster is listed without the rejected face.
    assert!(!c.clusters[0].faces.contains(&"5_a".to_string()));
}

#[test]
fn cgroup_json_like_the_web_app_writes_survives_a_rewrite() {
    // `JSON.stringify` of the web app's CGroupUserEntityData, plus keys a
    // newer client added (top level and inside a cluster) and the mobile
    // app's extra fields.
    let fixture = json!({
        "name": "Ana",
        "assigned": [
            {"id": "cluster_V1StGXR8Z5jdHi6BmyT", "faces": ["101_0.10000_0.20000_0.30000_0.40000"],
             "origin": "web"},
            null
        ],
        "rejectedFaceIDs": ["102_0.10000_0.20000_0.30000_0.40000"],
        "manuallyAssigned": [103, "104"],
        "isHidden": false,
        "avatarFaceID": "101_0.10000_0.20000_0.30000_0.40000",
        "isPinned": true,
        "hideFromMemories": false,
        "birthDate": "1990-05-17",
        "email": null,
        "userID": null,
        "spouse": {"cgroup": "cg_123"}
    });
    let mut d = CGroupData::from_value(fixture.clone());
    assert_eq!(d.name(), Some("Ana"));
    assert!(d.is_pinned() && !d.is_ignored());
    assert_eq!(d.manually_assigned(), vec![103, 104]);
    assert_eq!(
        d.avatar_face_id(),
        Some("101_0.10000_0.20000_0.30000_0.40000")
    );
    assert_eq!(d.assigned().len(), 1);
    d.set_name("Ana Maria");
    let mut assigned = d.assigned();
    assigned[0].faces.push("105_0.1_0.1_0.2_0.2".into());
    d.set_assigned(assigned);
    let v = d.to_value();
    assert_eq!(v["name"], "Ana Maria");
    assert_eq!(v["spouse"], fixture["spouse"]);
    assert_eq!(v["assigned"][0]["origin"], "web");
    assert_eq!(v["assigned"][0]["faces"].as_array().unwrap().len(), 2);
    for k in [
        "rejectedFaceIDs",
        "manuallyAssigned",
        "isPinned",
        "birthDate",
        "email",
        "userID",
        "avatarFaceID",
    ] {
        assert_eq!(v[k], fixture[k], "{k}");
    }
    // A minimal mobile-style object gets the keys the web parser requires.
    let v = CGroupData::from_value(json!({"name": "X"})).to_value();
    assert_eq!(v["isHidden"], false);
    assert_eq!(v["assigned"], json!([]));
}

// ------------------------------------------------------------ end to end

const EMAIL: &str = "faces@example.org";
const PASSWORD: &str = "correct horse battery staple";

fn landmarks() -> Value {
    json!([{"x": 0.3, "y": 0.3}, {"x": 0.7, "y": 0.3}, {"x": 0.5, "y": 0.5},
           {"x": 0.35, "y": 0.7}, {"x": 0.65, "y": 0.7}])
}

/// A face on file `file_id`, in Ente's `mldata` shape.
fn face(file_id: i64, slot: usize, emb: &[f32]) -> (String, Value) {
    let x = 0.1 + 0.4 * slot as f64;
    let id = format!("{file_id}_{x:.5}_0.20000_{:.5}_0.60000", x + 0.3);
    let v = json!({
        "faceID": id,
        "detection": {"box": {"x": x, "y": 0.2, "width": 0.3, "height": 0.4},
                      "landmarks": landmarks()},
        "score": 0.95,
        "blur": 300.0,
        "embedding": emb,
    });
    (id, v)
}

fn mldata(faces: &[Value]) -> String {
    json!({
        "face": {"version": 1, "client": "test/1", "width": 640, "height": 480, "faces": faces},
        "clip": {"version": 1, "client": "test/1", "embedding": [0.0, 1.0]},
    })
    .to_string()
}

/// The server's cgroup entities, decrypted independently of the core,
/// exactly as an Ente app would.
async fn server_cgroups(api: &Api, master_key: &[u8]) -> Vec<(String, Option<Value>, String)> {
    let k: Value = api
        .get_json("/user-entity/key", &[("type", "cgroup".to_string())])
        .await
        .unwrap();
    let key = crypto::secretbox_open(
        &crypto::unb64(k["encryptedKey"].as_str().unwrap()).unwrap(),
        &crypto::unb64(k["header"].as_str().unwrap()).unwrap(),
        master_key,
    )
    .unwrap();
    let diff: Value = api
        .get_json(
            "/user-entity/entity/diff",
            &[
                ("type", "cgroup".to_string()),
                ("sinceTime", "0".to_string()),
                ("limit", "500".to_string()),
            ],
        )
        .await
        .unwrap();
    diff["diff"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let id = e["id"].as_str().unwrap().to_string();
            if e["isDeleted"].as_bool().unwrap() {
                return (id, None, String::new());
            }
            let enc = e["encryptedData"].as_str().unwrap().to_string();
            let gz = crypto::decrypt_blob(
                &crypto::unb64(&enc).unwrap(),
                &crypto::unb64(e["header"].as_str().unwrap()).unwrap(),
                &key,
            )
            .unwrap();
            // cgroups are gzipped JSON.
            let mut s = String::new();
            flate2::read::GzDecoder::new(gz.as_slice())
                .read_to_string(&mut s)
                .unwrap();
            (id, Some(serde_json::from_str(&s).unwrap()), enc)
        })
        .collect()
}

fn find<'a>(people: &'a [Person], name: &str) -> &'a Person {
    people
        .iter()
        .find(|p| p.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no {name} in {people:?}"))
}

fn names(faces: &[FileFace]) -> Vec<Option<String>> {
    faces.iter().map(|f| f.person_name.clone()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn people_flows_against_the_real_server() {
    let srv = server(&[EMAIL]).await;
    let acc = signup(&srv.url, EMAIL, PASSWORD).await;
    let api = Api::new(&srv.url).with_token(acc.token.clone());
    let t = TestCore::connected(&srv.url).await;
    let core = &t.core;
    core.login(EMAIL.into(), PASSWORD.into()).await.unwrap();

    // Six photos.
    for i in 0..6u8 {
        let p = t.dir.path().join(format!("IMG_{i}.jpg"));
        std::fs::write(&p, jpeg(640, 480, 10 + i * 30)).unwrap();
        core.enqueue_upload(p.display().to_string(), None).unwrap();
    }
    let st = core.run_uploads().await.unwrap();
    assert_eq!(st.failed, 0, "{st:?}");
    core.sync(None).await.unwrap();
    let mut ids: Vec<i64> = core.timeline_page(0, 50).iter().map(|i| i.id).collect();
    ids.sort();
    assert_eq!(ids.len(), 6);

    // Ana on files 0–3, Bo on 0, 2, 4, and on 5 someone who looks a bit
    // like Ana (two faces: a separate cluster, suggested for Ana).
    let mut rng = Rng(42);
    let (ana, bo) = (rng.unit(), rng.unit());
    // A direction orthogonal to Ana's, so the look-alike sits at ~0.7.
    let other = rng.unit();
    let other = unit(
        other
            .iter()
            .zip(&ana)
            .map(|(o, a)| o - dot(&other, &ana) * a)
            .collect(),
    );
    let mut ana_faces = Vec::new();
    let mut bo_faces = Vec::new();
    let mut like_faces = Vec::new();
    let mut per_file: Vec<Vec<Value>> = Vec::new();
    for (n, &fid) in ids.iter().enumerate() {
        let mut faces = Vec::new();
        if n < 4 {
            let (id, v) = face(fid, 0, &mix(&mut rng, &[(1.0, &ana)], 0.25));
            ana_faces.push(id);
            faces.push(v);
        }
        if n % 2 == 0 && n < 5 {
            let (id, v) = face(fid, 1, &mix(&mut rng, &[(1.0, &bo)], 0.25));
            bo_faces.push(id);
            faces.push(v);
        }
        if n == 5 {
            for slot in 0..2 {
                let e = mix(&mut rng, &[(0.7, &ana), (0.71, &other)], 0.1);
                let (id, v) = face(fid, slot, &e);
                like_faces.push(id);
                faces.push(v);
            }
        }
        core.ml_testing_put_mldata(fid, mldata(&faces)).unwrap();
        per_file.push(faces);
    }
    core.ml_testing_refresh_faces().await.unwrap();

    let people = core.people(false, 0);
    assert_eq!(people.len(), 3, "{people:?}");
    assert!(
        people
            .iter()
            .all(|p| p.name.is_none() && p.id.starts_with("cluster:"))
    );
    let ana_cluster = people.iter().find(|p| p.file_count == 4).unwrap().clone();
    assert_eq!(core.people(false, 4).len(), 1, "min_files filters clusters");
    let cover = ana_cluster.cover.clone().unwrap();
    assert!(ana_faces.contains(&cover.face_id));
    assert!((cover.width - 0.3).abs() < 1e-6 && (cover.height - 0.4).abs() < 1e-6);

    // Name Ana: a cgroup on the server, encrypted with the cgroup entity key.
    const ANA: &str = "Anastasia Quokka";
    let p = core
        .name_person(ana_cluster.id.clone(), ANA.into())
        .await
        .unwrap();
    assert_eq!(p.name.as_deref(), Some(ANA));
    assert!(!p.id.starts_with("cluster:"));
    assert_eq!(p.file_count, 4);
    let ana_id = p.id.clone();
    let remote = server_cgroups(&api, &acc.master_key).await;
    assert_eq!(remote.len(), 1);
    let (rid, data, enc) = &remote[0];
    assert_eq!(rid, &ana_id);
    assert!(
        !enc.contains(ANA) && !String::from_utf8_lossy(&crypto::unb64(enc).unwrap()).contains(ANA)
    );
    let data = data.as_ref().unwrap();
    assert_eq!(data["name"], ANA);
    assert_eq!(data["isHidden"], false);
    let mut assigned: Vec<String> = data["assigned"][0]["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect();
    assigned.sort();
    let mut want = ana_faces.clone();
    want.sort();
    assert_eq!(assigned, want);
    assert_eq!(
        format!("cluster:{}", data["assigned"][0]["id"].as_str().unwrap()),
        ana_cluster.id
    );

    // Nothing readable in the local store either.
    let mut hay = std::fs::read(t.data_dir().join("photovault-core.sqlite")).unwrap();
    hay.extend(std::fs::read(t.data_dir().join("photovault-core.sqlite-wal")).unwrap_or_default());
    assert!(!hay.windows(ANA.len()).any(|w| w == ANA.as_bytes()));

    // The look-alike cluster is suggested for Ana.
    let sugg = core.person_suggestions(ana_id.clone());
    assert_eq!(sugg.len(), 1, "{sugg:?}");
    let like_cluster = sugg[0].id.clone();
    assert_eq!(sugg[0].file_count, 1);

    // A person created by another Ente client (web shape, unknown keys).
    let key = {
        let k: Value = api
            .get_json("/user-entity/key", &[("type", "cgroup".to_string())])
            .await
            .unwrap();
        photovault_core::ml::entity::open_entity_key(
            k["encryptedKey"].as_str().unwrap(),
            k["header"].as_str().unwrap(),
            &acc.master_key,
        )
        .unwrap()
    };
    let web = json!({
        "name": "Bo Web",
        "assigned": [{"id": "cluster_FromTheWebApp000000", "faces": bo_faces}],
        "rejectedFaceIDs": [],
        "manuallyAssigned": [],
        "isHidden": false,
        "isPinned": false,
        "hideFromMemories": false,
        "futureKey": {"keep": [1, 2, 3]},
    });
    let (d, h) = photovault_core::ml::entity::encrypt_data("cgroup", &web, &key).unwrap();
    let created: Value = api
        .json(
            Method::POST,
            "/user-entity/entity",
            &json!({"type": "cgroup", "encryptedData": d, "header": h}),
        )
        .await
        .unwrap();
    let bo_id = created["id"].as_str().unwrap().to_string();
    core.ml_testing_refresh_faces().await.unwrap();
    let people = core.people(false, 0);
    let bo = find(&people, "Bo Web");
    assert_eq!(bo.id, bo_id);
    assert_eq!(bo.file_count, 3);
    // Bo's local cluster is gone: the remote assignment wins.
    assert_eq!(people.len(), 3, "{people:?}");
    assert_eq!(
        names(&core.file_faces(ids[0])),
        vec![Some(ANA.to_string()), Some("Bo Web".to_string())]
    );

    // Rename + birth date keep the other client's keys.
    core.name_person(bo_id.clone(), "Robert Web".into())
        .await
        .unwrap();
    core.set_person_birth_date(bo_id.clone(), Some("1990-05-17".into()))
        .await
        .unwrap();
    assert!(
        core.set_person_birth_date(bo_id.clone(), Some("17.05.1990".into()))
            .await
            .is_err()
    );
    let remote = server_cgroups(&api, &acc.master_key).await;
    let bo_remote = remote
        .iter()
        .find(|r| r.0 == bo_id)
        .and_then(|r| r.1.clone())
        .unwrap();
    assert_eq!(bo_remote["name"], "Robert Web");
    assert_eq!(bo_remote["birthDate"], "1990-05-17");
    assert_eq!(bo_remote["futureKey"], json!({"keep": [1, 2, 3]}));
    assert_eq!(
        bo_remote["assigned"][0]["id"],
        "cluster_FromTheWebApp000000"
    );
    assert_eq!(
        core.person(bo_id.clone()).unwrap().birth_date.as_deref(),
        Some("1990-05-17")
    );

    // Hide / unhide.
    core.set_person_hidden(bo_id.clone(), true).await.unwrap();
    assert!(core.people(false, 0).iter().all(|p| p.id != bo_id));
    assert!(
        core.people(true, 0)
            .iter()
            .any(|p| p.id == bo_id && p.is_hidden)
    );
    core.set_person_hidden(bo_id.clone(), false).await.unwrap();
    assert!(core.people(false, 0).iter().any(|p| p.id == bo_id));

    // Files and cover.
    let files = core.person_files(ana_id.clone());
    assert_eq!(files.len(), 4);
    let timeline: Vec<i64> = core.timeline_page(0, 50).iter().map(|i| i.id).collect();
    let pos = |id: i64| timeline.iter().position(|t| *t == id).unwrap();
    assert!(
        files.windows(2).all(|w| pos(w[0].id) < pos(w[1].id)),
        "newest first"
    );
    core.set_person_cover(ana_id.clone(), ana_faces[2].clone())
        .await
        .unwrap();
    assert_eq!(
        core.person(ana_id.clone()).unwrap().cover.unwrap().face_id,
        ana_faces[2]
    );
    assert!(
        core.set_person_cover(ana_id.clone(), bo_faces[0].clone())
            .await
            .is_err()
    );

    // Face thumbnail: a square JPEG, then the same bytes from the encrypted cache.
    let thumb = core.face_thumbnail(ana_faces[0].clone()).await.unwrap();
    assert_eq!(&thumb[..2], &[0xFF, 0xD8]);
    let img = image::load_from_memory(&thumb).unwrap();
    assert_eq!(img.width(), img.height());
    assert_eq!(
        core.face_thumbnail(ana_faces[0].clone()).await.unwrap(),
        thumb
    );
    let cached: Vec<_> = std::fs::read_dir(t.cache_dir().join("faces"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(cached.len(), 1);
    let blob = std::fs::read(cached[0].path()).unwrap();
    assert_ne!(&blob[..2], &[0xFF, 0xD8]);
    assert!(
        !blob.windows(64).any(|w| w == &thumb[..64]),
        "cached encrypted"
    );

    // Accept the suggestion: Ana gains file 5.
    core.answer_suggestion(ana_id.clone(), like_cluster.clone(), true)
        .await
        .unwrap();
    assert_eq!(core.person(ana_id.clone()).unwrap().file_count, 5);
    assert!(core.person(like_cluster.clone()).is_none());
    assert!(core.person_suggestions(ana_id.clone()).is_empty());

    // Remove a face: rejected on the server, and not suggested back.
    core.remove_faces_from_person(ana_id.clone(), vec![ana_faces[3].clone()])
        .await
        .unwrap();
    assert_eq!(core.person(ana_id.clone()).unwrap().file_count, 4);
    let remote = server_cgroups(&api, &acc.master_key).await;
    let ana_remote = remote
        .iter()
        .find(|r| r.0 == ana_id)
        .and_then(|r| r.1.clone())
        .unwrap();
    assert!(
        ana_remote["rejectedFaceIDs"]
            .as_array()
            .unwrap()
            .contains(&json!(ana_faces[3]))
    );
    let removed_owner = core
        .file_faces(ids[3])
        .into_iter()
        .find(|f| f.face.face_id == ana_faces[3])
        .unwrap();
    assert_ne!(removed_owner.person_id.as_deref(), Some(ana_id.as_str()));
    assert!(
        core.person_suggestions(ana_id.clone())
            .iter()
            .all(|p| Some(&p.id) != removed_owner.person_id.as_ref())
    );

    // Merge Robert into Ana: one person, Robert deleted on the server.
    let merged = core
        .merge_people(ana_id.clone(), bo_id.clone())
        .await
        .unwrap();
    assert_eq!(merged.id, ana_id);
    assert_eq!(merged.file_count, 5);
    assert!(core.person(bo_id.clone()).is_none());
    let remote = server_cgroups(&api, &acc.master_key).await;
    assert!(remote.iter().any(|r| r.0 == bo_id && r.1.is_none()));

    // Delete Ana: the faces become unnamed clusters again.
    core.delete_person(ana_id.clone()).await.unwrap();
    let people = core.people(true, 0);
    assert!(people.iter().all(|p| p.name.is_none()), "{people:?}");
    assert!(!people.is_empty());
    let file0 = core.file_faces(ids[0]);
    assert!(file0.iter().all(|f| {
        f.person_id
            .as_deref()
            .is_some_and(|p| p.starts_with("cluster:"))
    }));

    // Hiding an unnamed cluster ignores it (a nameless hidden cgroup).
    let c = people[0].id.clone();
    core.set_person_hidden(c.clone(), true).await.unwrap();
    assert!(core.people(false, 0).iter().all(|p| p.id != c));
    let hidden: Vec<Person> = core
        .people(true, 0)
        .into_iter()
        .filter(|p| p.is_hidden)
        .collect();
    assert_eq!(hidden.len(), 1);
    let remote = server_cgroups(&api, &acc.master_key).await;
    assert!(remote.iter().any(|r| {
        r.0 == hidden[0].id
            && r.1
                .as_ref()
                .is_some_and(|d| d["name"] == "" && d["isHidden"] == true)
    }));
    // Unhiding it brings the cluster back.
    core.set_person_hidden(hidden[0].id.clone(), false)
        .await
        .unwrap();
    assert!(core.people(true, 0).iter().all(|p| !p.is_hidden));

    // A new face of a named person joins its cgroup on the server (Ente's
    // reconcileClusters).
    let ana_again = core
        .people(true, 0)
        .into_iter()
        .find(|p| {
            p.cover
                .as_ref()
                .is_some_and(|c| ana_faces[..3].contains(&c.face_id))
        })
        .unwrap();
    let cleo = core
        .name_person(ana_again.id.clone(), "Cleo".into())
        .await
        .unwrap();
    assert_eq!(cleo.file_count, 3);
    let mut f4 = per_file[4].clone();
    let (new_face, v) = face(ids[4], 2, &mix(&mut rng, &[(1.0, &ana)], 0.25));
    f4.push(v);
    core.ml_testing_put_mldata(ids[4], mldata(&f4)).unwrap();
    core.ml_testing_refresh_faces().await.unwrap();
    assert_eq!(core.person(cleo.id.clone()).unwrap().file_count, 4);
    let remote = server_cgroups(&api, &acc.master_key).await;
    let cleo_remote = remote
        .iter()
        .find(|r| r.0 == cleo.id)
        .and_then(|r| r.1.clone())
        .unwrap();
    assert!(
        cleo_remote["assigned"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["faces"].as_array().unwrap().contains(&json!(new_face))),
        "{cleo_remote}"
    );

    // A second device sees the same people after a refresh.
    let t2 = TestCore::connected(&srv.url).await;
    t2.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap();
    t2.core.ml_testing_refresh_faces().await.unwrap();
    assert!(
        t2.core.people(true, 0).is_empty(),
        "no mldata on this device yet"
    );
}
