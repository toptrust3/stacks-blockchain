use p256k1::point::Point;
use p256k1::scalar::Scalar;
use stacks_signer::signing_round::{DkgBegin, MessageTypes, SignatureShare, SigningRound};

fn setup_signer(total: usize, threshold: usize) -> SigningRound {
    let my_id = 1;
    let mut signer = SigningRound::new(my_id, threshold, total);
    signer.reset();
    signer
}

#[test]
fn dkg_begin() {
    let total = 2;
    let mut signer = setup_signer(total, total - 1);
    assert_eq!(signer.commitments.len(), 0);

    let dkg_begin_msg = MessageTypes::DkgBegin(DkgBegin { id: [0; 32] });
    let msgs = signer.process(dkg_begin_msg).unwrap();
    assert_eq!(msgs.len(), total);

    // part of the DKG_BEGIN process is to fill the commitments array
    assert_eq!(signer.commitments.len(), signer.total);
}

#[test]
fn signature_share() {
    let share: frost::common::SignatureShare<Point> = frost::common::SignatureShare {
        id: 0,
        z_i: Scalar::new(),
        public_key: Default::default(),
    };

    let msg_share = MessageTypes::SignatureShare(SignatureShare {
        signature_shares: vec![share.z_i],
    });

    let mut signer = setup_signer(2, 1);
    signer.process(msg_share).unwrap();
}