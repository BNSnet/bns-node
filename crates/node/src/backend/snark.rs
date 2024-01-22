//! SNARK Backend
//! ================

use std::sync::Arc;

use dashmap::DashMap;
use rings_core::message::MessagePayload;
use rings_derive::wasm_export;
use rings_rpc::method::Method;
use rings_snark::prelude::nova::provider;
use rings_snark::prelude::nova::provider::ipa_pc;
use rings_snark::prelude::nova::provider::mlkzg;
use rings_snark::prelude::nova::spartan;
use rings_snark::prelude::ff;
use rings_snark::prelude::nova::traits::snark::RelaxedR1CSSNARKTrait;
use rings_snark::prelude::nova::traits::Engine;
use rings_snark::circuit;
use rings_snark::r1cs;
use rings_snark::snark::CompressedSNARK;
use rings_snark::snark::ProverKey;
use rings_snark::snark::PublicParams;
use rings_snark::snark::VerifierKey;
use rings_snark::snark::SNARK;
use serde::Deserialize;
use serde::Serialize;

use super::types::SNARKProofTask;
use super::types::SNARKTask;
use super::types::SNARKTaskMessage;
use super::types::SNARKVerifyTask;
use crate::backend::types::BackendMessage;
use crate::backend::types::MessageHandler;
use crate::error::Error;
use crate::error::Result;
use crate::provider::Provider;

type TaskId = uuid::Uuid;
/// Behaviour of SNARK provier and verifier
#[wasm_export]
#[derive(Default, Clone)]
pub struct SNARKBehaviour {
    /// map of task_id and task
    task: DashMap<TaskId, SNARKProofTask>,
    /// map of task_id and result
    verified: DashMap<TaskId, bool>,
}

#[wasm_export]
impl SNARKBehaviour {
    pub fn new() -> SNARKBehaviour {
	Self::default()
    }
}

/// Types for circuit
pub enum CircuitGenerator {
    /// Circuit based on Vesta curve
    Vesta(circuit::WasmCircuitGenerator<<provider::VestaEngine as Engine>::Base>),
    /// Circuit based on pallas curve
    Pallas(circuit::WasmCircuitGenerator<<provider::PallasEngine as Engine>::Base>),
    /// Circuit based on KZG bn256
    Bn256KZG(circuit::WasmCircuitGenerator<<provider::mlkzg::Bn256EngineKZG as Engine>::Base>)
}

/// Supported prime field
#[wasm_export]
pub enum SupportedPrimeField {
    /// field of vesta curve
    Vesta,
    /// field of pallas curve
    Pallas,
    /// bn256 with kzg
    Bn256KZG
}

/// Supported prime field
pub enum FieldEnum {
    /// field of vesta curve
    Vesta(<provider::VestaEngine as Engine>::Base),
    /// field of pallas curve
    Pallas(<provider::PallasEngine as Engine>::Base),
    /// bn256 with kzg
    Bn256KZG(<provider::mlkzg::Bn256EngineKZG as Engine>::Base)
}

/// Input type
pub type Input = Vec<(String, Vec<Field>)>;

/// Field type
#[wasm_export]
pub struct Field {
    value: FieldEnum
}

/// Types of Circuit
pub enum CircuitEnum {
    /// Based on vesta curve
    Vesta(circuit::Circuit<<provider::VestaEngine as Engine>::Base>),
    /// Based on pallas curve
    Pallas(circuit::Circuit<<provider::PallasEngine as Engine>::Base>),
    /// based on bn256 and KZG
    Bn256KZG(circuit::Circuit<<provider::mlkzg::Bn256EngineKZG as Engine>::Base>)
}

/// Circuit, it's a typeless wrapper of rings_snark circuit
#[wasm_export]
pub struct Circuit {
    inner: CircuitEnum
}


#[wasm_export]
impl Field {
    /// create field from u64
    pub fn from_u64(v: u64, ty: SupportedPrimeField) -> Self {
	match ty {
	    SupportedPrimeField::Vesta => Self {
		value:  FieldEnum::Vesta(
		    <provider::VestaEngine as Engine>::Base::from(v)
		)
	    },
	    SupportedPrimeField::Pallas => Self {
		value:  FieldEnum::Pallas(
		    <provider::PallasEngine as Engine>::Base::from(v)
		)
	    },
	    SupportedPrimeField::Bn256KZG => Self {
		value:  FieldEnum::Bn256KZG(
		    <provider::mlkzg::Bn256EngineKZG as Engine>::Base::from(v)
		)
	    }

	}
    }
}


pub struct SNARKTaskBuilder {
    circuit_generator: CircuitGenerator,
}

impl SNARKTaskBuilder {
    pub async fn from_local(r1cs_path: String, witness_wasm_path: String, field: SupportedPrimeField) -> Result<Self> {
	match field {
	    SupportedPrimeField::Vesta => {
		type F = <provider::VestaEngine as Engine>::Base;
		let r1cs = r1cs::load_r1cs::<F>(
		    r1cs::Path::Local(r1cs_path),
		    r1cs::Format::Bin
		).await?;
		let witness_calculator = r1cs::load_circom_witness_calculator(
		    r1cs::Path::Local(witness_wasm_path)
		).await?;
		let circuit_generator = circuit::WasmCircuitGenerator::<F>::new(r1cs, witness_calculator);
		Ok(Self {
		    circuit_generator:  CircuitGenerator::Vesta(circuit_generator)
		})
	    },
	    SupportedPrimeField::Pallas => {
		type F = <provider::PallasEngine as Engine>::Base;
		let r1cs = r1cs::load_r1cs::<F>(
		    r1cs::Path::Local(r1cs_path),
		    r1cs::Format::Bin
		).await?;
		let witness_calculator = r1cs::load_circom_witness_calculator(
		    r1cs::Path::Local(witness_wasm_path)
		).await?;
		let circuit_generator = circuit::WasmCircuitGenerator::<F>::new(r1cs, witness_calculator);
		Ok(Self {
		    circuit_generator:  CircuitGenerator::Pallas(circuit_generator)
		})
	    }
	    SupportedPrimeField::Bn256KZG => {
		type F = <provider::mlkzg::Bn256EngineKZG as Engine>::Base;
		let r1cs = r1cs::load_r1cs::<F>(
		    r1cs::Path::Local(r1cs_path),
		    r1cs::Format::Bin
		).await?;
		let witness_calculator = r1cs::load_circom_witness_calculator(
		    r1cs::Path::Local(witness_wasm_path)
		).await?;
		let circuit_generator = circuit::WasmCircuitGenerator::<F>::new(r1cs, witness_calculator);
		Ok(Self {
		    circuit_generator:  CircuitGenerator::Bn256KZG(circuit_generator)
		})
	    }
	}
    }

    /// generate recursive circuits
    pub async fn gen_circuits(&self, public_input: Input, private_inputs: Vec<Input>, round: usize) -> Result<Vec<Circuit>> {
	match &self.circuit_generator {
	    CircuitGenerator::Vesta(g) => {
		type F = <provider::VestaEngine as Engine>::Base;

		let input: circuit::Input<F> = public_input.into_iter().map(|(s, v)| {
		    (s, v.into_iter().map(|inp| {
			if let FieldEnum::Vesta(x) = inp.value {
			    x
			} else {
			    panic!("Wrong curve, expect Vesta")
			}
		    }).collect())
		}).collect::<Vec<(String, Vec<F>)>>().into();

		let private_inputs: Vec<circuit::Input<F>> = private_inputs.into_iter().map(|inp| {
		    inp.into_iter().map(|(s, v)| {
			let fields: Vec<F> = v.into_iter().map(|inp| {
			    if let FieldEnum::Vesta(x) = inp.value {
				x
			    } else {
				panic!("Wrong curve, expect Vesta")
			    }
			}).collect();
			(s, fields)
		    }).collect::<Vec<(String, Vec<F>)>>().into()
		}).collect();


		let circuits = g.gen_recursive_circuit(
		    input.into(), private_inputs, round, true
		)?.iter().map(|c| Circuit {inner: CircuitEnum::Vesta(c.clone())}).collect::<Vec<Circuit>>();
		Ok(circuits)
	    }
	    CircuitGenerator::Pallas(g) => {
		type F = <provider::PallasEngine as Engine>::Base;

		let input: circuit::Input<F> = public_input.into_iter().map(|(s, v)| {
		    (s, v.into_iter().map(|inp| {
			if let FieldEnum::Pallas(x) = inp.value {
			    x
			} else {
			    panic!("Wrong curve, expect pallas")
			}
		    }).collect())
		}).collect::<Vec<(String, Vec<F>)>>().into();

		let private_inputs: Vec<circuit::Input<F>> = private_inputs.into_iter().map(|inp| {
		    inp.into_iter().map(|(s, v)| {
			let fields: Vec<F> = v.into_iter().map(|inp| {
			    if let FieldEnum::Pallas(x) = inp.value {
				x
			    } else {
				panic!("Wrong curve, expect Vesta")
			    }
			}).collect();
			(s, fields)
		    }).collect::<Vec<(String, Vec<F>)>>().into()
		}).collect();

		let circuits = g.gen_recursive_circuit(
		    input.into(), private_inputs, round, true
		)?.iter().map(|c| Circuit {inner: CircuitEnum::Pallas(c.clone())}).collect::<Vec<Circuit>>();
		Ok(circuits)
	    }
	    CircuitGenerator::Bn256KZG(g) => {
		type F = <provider::mlkzg::Bn256EngineKZG as Engine>::Base;

		let input: circuit::Input<F> = public_input.into_iter().map(|(s, v)| {
		    (s, v.into_iter().map(|inp| {
			if let FieldEnum::Bn256KZG(x) = inp.value {
			    x
			} else {
			    panic!("Wrong curve, expect bn256")
			}
		    }).collect())
		}).collect::<Vec<(String, Vec<F>)>>().into();

		let private_inputs: Vec<circuit::Input<F>> = private_inputs.into_iter().map(|inp| {
		    inp.into_iter().map(|(s, v)| {
			let fields: Vec<F> = v.into_iter().map(|inp| {
			    if let FieldEnum::Bn256KZG(x) = inp.value {
				x
			    } else {
				panic!("Wrong curve, expect bn256")
			    }
			}).collect();
			(s, fields)
		    }).collect::<Vec<(String, Vec<F>)>>().into()
		}).collect();

		let circuits = g.gen_recursive_circuit(
		    input.into(), private_inputs, round, true
		)?.iter().map(|c| Circuit {inner: CircuitEnum::Bn256KZG(c.clone())}).collect::<Vec<Circuit>>();
		Ok(circuits)
	    }
	}
    }
}





/// SNARK Proof
#[derive(Serialize, Deserialize)]
pub struct SNARKProof<E1, E2, S1, S2>
where
    S1: RelaxedR1CSSNARKTrait<E1>,
    S2: RelaxedR1CSSNARKTrait<E2>,
    E1: Engine<Base = <E2 as Engine>::Scalar>,
    E2: Engine<Base = <E1 as Engine>::Scalar>,
{
    /// verifier key of proof
    #[serde(
        serialize_with = "crate::util::serialize_forward",
        deserialize_with = "crate::util::deserialize_forward"
    )]
    pub vk: VerifierKey<E1, E2, S1, S2>,
    #[serde(
        serialize_with = "crate::util::serialize_forward",
        deserialize_with = "crate::util::deserialize_forward"
    )]
    /// compressed proof
    pub proof: CompressedSNARK<E1, E2, S1, S2>,
}

/// SNARK proof generator, including setup, proof and verify
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SNARKGenerator<E1, E2>
where
    E1: Engine<Base = <E2 as Engine>::Scalar>,
    E2: Engine<Base = <E1 as Engine>::Scalar>,
{
    snark: SNARK<E1, E2>,
    circuits: Vec<circuit::Circuit<<E1 as Engine>::Scalar>>,
    pp: PublicParams<E1, E2>,
}

impl<E1, E2> SNARKGenerator<E1, E2>
where
    E1: Engine<Base = <E2 as Engine>::Scalar>,
    E2: Engine<Base = <E1 as Engine>::Scalar>,
{
    /// Setup snark, get pk and vk
    pub fn fold(&mut self) -> Result<()> {
        Ok(self.snark.fold_all(&self.pp, &self.circuits)?)
    }

    /// setup compressed snark, get (pk, vk)
    #[allow(clippy::type_complexity)]
    pub fn setup<S1: RelaxedR1CSSNARKTrait<E1>, S2: RelaxedR1CSSNARKTrait<E2>>(
        &self,
    ) -> Result<(ProverKey<E1, E2, S1, S2>, VerifierKey<E1, E2, S1, S2>)> {
        Ok(SNARK::<E1, E2>::compress_setup(&self.pp)?)
    }

    /// gen proof for compressed snark
    pub fn prove<S1: RelaxedR1CSSNARKTrait<E1>, S2: RelaxedR1CSSNARKTrait<E2>>(
        &self,
        pk: impl AsRef<ProverKey<E1, E2, S1, S2>>,
    ) -> Result<CompressedSNARK<E1, E2, S1, S2>> {
        Ok(self.snark.compress_prove(&self.pp, pk)?)
    }

    /// verify a proof
    #[allow(clippy::type_complexity)]
    pub fn verify<S1: RelaxedR1CSSNARKTrait<E1>, S2: RelaxedR1CSSNARKTrait<E2>>(
        &self,
        proof: impl AsRef<CompressedSNARK<E1, E2, S1, S2>>,
        vk: impl AsRef<VerifierKey<E1, E2, S1, S2>>,
    ) -> Result<(Vec<E1::Scalar>, Vec<E2::Scalar>)> {
        let steps = self.circuits.len();
        let first_input = self.circuits.first().unwrap().get_public_inputs();
        Ok(SNARK::<E1, E2>::compress_verify(
            proof,
            vk,
            steps,
            first_input,
        )?)
    }
}

impl SNARKBehaviour {
    fn handle_snark_proof_task(data: SNARKProofTask) -> Result<SNARKVerifyTask> {
        match data {
            SNARKProofTask::VastaPallas(s) => {
                type E1 = provider::VestaEngine;
                type E2 = provider::PallasEngine;
                type EE1 = ipa_pc::EvaluationEngine<E1>;
                type EE2 = ipa_pc::EvaluationEngine<E2>;
                type S1 = spartan::snark::RelaxedR1CSSNARK<E1, EE1>;
                type S2 = spartan::snark::RelaxedR1CSSNARK<E2, EE2>;
                let (pk, vk) = s.setup()?;
                let compressed_proof = s.prove::<S1, S2>(&pk)?;
                let proof = SNARKProof::<E1, E2, S1, S2> {
                    vk,
                    proof: compressed_proof,
                };
                Ok(SNARKVerifyTask::VastaPallas(serde_json::to_string(&proof)?))
            }
            SNARKProofTask::PallasVasta(s) => {
                type E1 = provider::PallasEngine;
                type E2 = provider::VestaEngine;
                type EE1 = ipa_pc::EvaluationEngine<E1>;
                type EE2 = ipa_pc::EvaluationEngine<E2>;
                type S1 = spartan::snark::RelaxedR1CSSNARK<E1, EE1>;
                type S2 = spartan::snark::RelaxedR1CSSNARK<E2, EE2>;
                let (pk, vk) = s.setup()?;
                let compressed_proof = s.prove::<S1, S2>(&pk)?;
                let proof = SNARKProof::<E1, E2, S1, S2> {
                    vk,
                    proof: compressed_proof,
                };
                Ok(SNARKVerifyTask::PallasVasta(serde_json::to_string(&proof)?))
            }
            SNARKProofTask::Bn256KZGGrumpkin(s) => {
                type E1 = provider::mlkzg::Bn256EngineKZG;
                type E2 = provider::GrumpkinEngine;
                type EE1 = mlkzg::EvaluationEngine<E1>;
                type EE2 = ipa_pc::EvaluationEngine<E2>;
                type S1 = spartan::snark::RelaxedR1CSSNARK<E1, EE1>; // non-preprocessing SNARK
                type S2 = spartan::snark::RelaxedR1CSSNARK<E2, EE2>; // non-preprocessing SNARK
                let (pk, vk) = s.setup()?;
                let compressed_proof = s.prove::<S1, S2>(&pk)?;
                let proof = SNARKProof::<E1, E2, S1, S2> {
                    vk,
                    proof: compressed_proof,
                };
                Ok(SNARKVerifyTask::Bn256KZGGrumpkin(serde_json::to_string(
                    &proof,
                )?))
            }
        }
    }

    fn handle_snark_verify_task(data: SNARKVerifyTask, snark: SNARKProofTask) -> Result<bool> {
        match data {
            SNARKVerifyTask::PallasVasta(p) => {
                type E1 = provider::PallasEngine;
                type E2 = provider::VestaEngine;
                type EE1 = ipa_pc::EvaluationEngine<E1>;
                type EE2 = ipa_pc::EvaluationEngine<E2>;
                type S1 = spartan::snark::RelaxedR1CSSNARK<E1, EE1>;
                type S2 = spartan::snark::RelaxedR1CSSNARK<E2, EE2>;
                let proof = serde_json::from_str::<SNARKProof<E1, E2, S1, S2>>(&p)?;
                if let SNARKProofTask::PallasVasta(t) = snark {
                    let ret = t.verify::<S1, S2>(proof.proof, proof.vk);
                    Ok(ret.is_ok())
                } else {
                    Err(Error::SNARKCurveNotMatch())
                }
            }
            SNARKVerifyTask::VastaPallas(p) => {
                type E1 = provider::VestaEngine;
                type E2 = provider::PallasEngine;
                type EE1 = ipa_pc::EvaluationEngine<E1>;
                type EE2 = ipa_pc::EvaluationEngine<E2>;
                type S1 = spartan::snark::RelaxedR1CSSNARK<E1, EE1>;
                type S2 = spartan::snark::RelaxedR1CSSNARK<E2, EE2>;
                let proof = serde_json::from_str::<SNARKProof<E1, E2, S1, S2>>(&p)?;
                if let SNARKProofTask::VastaPallas(t) = snark {
                    let ret = t.verify::<S1, S2>(proof.proof, proof.vk);
                    Ok(ret.is_ok())
                } else {
                    Err(Error::SNARKCurveNotMatch())
                }
            }
            SNARKVerifyTask::Bn256KZGGrumpkin(p) => {
                type E1 = provider::mlkzg::Bn256EngineKZG;
                type E2 = provider::GrumpkinEngine;
                type EE1 = mlkzg::EvaluationEngine<E1>;
                type EE2 = ipa_pc::EvaluationEngine<E2>;
                type S1 = spartan::snark::RelaxedR1CSSNARK<E1, EE1>; // non-preprocessing SNARK
                type S2 = spartan::snark::RelaxedR1CSSNARK<E2, EE2>; // non-preprocessing SNARK
                let proof = serde_json::from_str::<SNARKProof<E1, E2, S1, S2>>(&p)?;
                if let SNARKProofTask::Bn256KZGGrumpkin(t) = snark {
                    let ret = t.verify::<S1, S2>(proof.proof, proof.vk);
                    Ok(ret.is_ok())
                } else {
                    Err(Error::SNARKCurveNotMatch())
                }
            }
        }
    }
}

impl From<SNARKGenerator<provider::PallasEngine, provider::VestaEngine>> for SNARKProofTask {
    fn from(snark: SNARKGenerator<provider::PallasEngine, provider::VestaEngine>) -> Self {
        Self::PallasVasta(snark)
    }
}

impl From<SNARKGenerator<provider::VestaEngine, provider::PallasEngine>> for SNARKProofTask {
    fn from(snark: SNARKGenerator<provider::VestaEngine, provider::PallasEngine>) -> Self {
        Self::VastaPallas(snark)
    }
}

impl From<SNARKGenerator<provider::mlkzg::Bn256EngineKZG, provider::GrumpkinEngine>>
    for SNARKProofTask
{
    fn from(
        snark: SNARKGenerator<provider::mlkzg::Bn256EngineKZG, provider::GrumpkinEngine>,
    ) -> Self {
        Self::Bn256KZGGrumpkin(snark)
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl MessageHandler<SNARKTaskMessage> for SNARKBehaviour {
    async fn handle_message(
        &self,
        provider: Arc<Provider>,
        ctx: &MessagePayload,
        msg: &SNARKTaskMessage,
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let verifier = ctx.relay.origin_sender();
        match &msg.task {
            SNARKTask::SNARKProof(t) => {
                let proof = Self::handle_snark_proof_task(t.clone())?;
                let resp: BackendMessage = SNARKTaskMessage {
                    task_id: msg.task_id,
                    task: SNARKTask::SNARKVerify(proof),
                }
                .into();
                let params = resp.into_send_backend_message_request(verifier)?;
                #[cfg(not(target_arch = "wasm32"))]
                provider.request(Method::SendBackendMessage, params).await?;
                #[cfg(target_arch = "wasm32")]
                {
                    let promise = provider.request(
                        Method::SendBackendMessage.to_string(),
                        rings_core::utils::js_value::serialize(&params)?,
                    );
                    wasm_bindgen_futures::JsFuture::from(promise)
                        .await
                        .map_err(|e| {
                            Error::JsError(format!("Failed send backend messate: {:?}", e))
                        })?;
                }
                Ok(())
            }
            SNARKTask::SNARKVerify(t) => {
                if let Some(task) = self.task.get(&msg.task_id) {
                    let verified = Self::handle_snark_verify_task(t.clone(), task.value().clone())?;
                    self.verified.insert(msg.task_id, verified);
                }
                Ok(())
            }
        }
    }
}

#[cfg_attr(target_family = "wasm", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait::async_trait)]
impl MessageHandler<BackendMessage> for SNARKBehaviour {
    async fn handle_message(
        &self,
        provider: Arc<Provider>,
        ctx: &MessagePayload,
        msg: &BackendMessage,
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        if let BackendMessage::SNARKTaskMessage(msg) = msg {
            Ok(self.handle_message(provider.clone(), ctx, msg).await?)
        } else {
            Ok(())
        }
    }
}