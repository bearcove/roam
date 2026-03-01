use afl::fuzz;

fn main() {
    fuzz!(|data: &[u8]| {
        let Ok(message) =
            roam::facet_postcard::from_slice_borrowed::<roam_types::Message<'_>>(data)
        else {
            return;
        };

        let _ = roam::facet_postcard::to_vec(&message);
    });
}
