pub mod xlstatus {
    pub mod v1 {
        tonic::include_proto!("xlstatus.v1");

        pub const FILE_DESCRIPTOR_SET: &[u8] =
            tonic::include_file_descriptor_set!("xlstatus_descriptor");
    }
}
