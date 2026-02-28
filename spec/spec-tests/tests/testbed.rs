use roam::RoamError;
use spec_proto::MathError;
use spec_tests::harness::{accept_subject, run_async};

// r[verify call.initiate]
// r[verify call.complete]
// r[verify call.lifecycle.single-response]
// r[verify call.lifecycle.ordering]
// r[verify transport.message.binary]
#[test]
fn rpc_echo_roundtrip() {
    run_async(async {
        let (client, mut child) = accept_subject().await?;
        let resp = client
            .echo("hello".to_string())
            .await
            .map_err(|e| format!("echo: {e:?}"))?;
        if resp.ret != "hello" {
            return Err(format!("expected \"hello\", got {:?}", resp.ret));
        }
        child.kill().await.ok();
        Ok::<_, String>(())
    })
    .unwrap();
}

// r[verify call.error.user]
#[test]
fn rpc_user_error_roundtrip() {
    run_async(async {
        let (client, mut child) = accept_subject().await?;
        let result = client.divide(10, 0).await;
        match result {
            Err(RoamError::User(MathError::DivisionByZero)) => {}
            Ok(resp) => {
                return Err(format!(
                    "expected Err(User(DivisionByZero)), got Ok({})",
                    resp.ret
                ));
            }
            Err(other) => {
                return Err(format!(
                    "expected Err(User(DivisionByZero)), got Err({other:?})"
                ));
            }
        }
        child.kill().await.ok();
        Ok::<_, String>(())
    })
    .unwrap();
}

// r[verify call.pipelining.allowed]
// r[verify call.pipelining.independence]
// r[verify core.call]
// r[verify core.call.request-id]
#[test]
fn rpc_pipelining_multiple_requests() {
    run_async(async {
        let (client, mut child) = accept_subject().await?;
        let (r1, r2, r3) = tokio::join!(
            client.echo("first".to_string()),
            client.echo("second".to_string()),
            client.echo("third".to_string()),
        );
        if r1.map_err(|e| format!("{e:?}"))?.ret != "first" {
            return Err("pipelining: first response wrong".to_string());
        }
        if r2.map_err(|e| format!("{e:?}"))?.ret != "second" {
            return Err("pipelining: second response wrong".to_string());
        }
        if r3.map_err(|e| format!("{e:?}"))?.ret != "third" {
            return Err("pipelining: third response wrong".to_string());
        }
        child.kill().await.ok();
        Ok::<_, String>(())
    })
    .unwrap();
}
