use rml_rtmp::chunk_io::ChunkDeserializer;
use rml_rtmp::messages::MessagePayload;
use rml_rtmp::messages::RtmpMessage;
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::io::Read;
use std::io::SeekFrom;

use clap::Parser;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    //#[clap(short, long)]
    input: String,
    output: Option<String>,
}

fn write_av_data(
    is_video: bool,
    message: &MessagePayload,
    payload: &[u8],
    file: &mut File,
) -> io::Result<()> {
    let tag_header = &mut [0_u8; 11];
    if is_video {
        tag_header[0] = 9;
    } else {
        tag_header[0] = 8;
    }

    println!("len: {}", payload.len());
    //data size
    tag_header[1] = (payload.len() >> 16 & 0xFF) as u8;
    tag_header[2] = (payload.len() >> 8 & 0xFF) as u8;
    tag_header[3] = (payload.len() & 0xFF) as u8;

    //timestamp
    let timestamp = message.timestamp.value;

    println!("timestamp: {}", timestamp);

    tag_header[4] = (timestamp >> 16 & 0xFF) as u8;
    tag_header[5] = (timestamp >> 8 & 0xFF) as u8;
    tag_header[6] = (timestamp & 0xFF) as u8;
    //timestamp ext
    tag_header[7] = (timestamp >> 24 & 0xFF) as u8;

    println!("tag_header: {:?}", tag_header);

    file.write_all(tag_header)?;

    //write payload
    file.write_all(payload)?;

    let tag_len = payload.len() + tag_header.len();
    let tag_len_slice = &mut [0_u8; 4];
    tag_len_slice[0] = (tag_len >> 24 & 0xFF) as u8;
    tag_len_slice[1] = (tag_len >> 16 & 0xFF) as u8;
    tag_len_slice[2] = (tag_len >> 8 & 0xFF) as u8;
    tag_len_slice[3] = (tag_len & 0xFF) as u8;

    file.write_all(tag_len_slice)?;

    Ok(())
}

fn main() -> io::Result<()> {
    println!("RTMP capture bin reader");
    println!("This reads raw binary bytes from a single direction in an RTMP stream");
    println!();

    let args = Args::parse();

    println!("input: {}", args.input);
    println!("output: {:?}", args.output);

    let handshake_len = 1 + 1536 + 1536;

    println!();

    let file_name = args.input;
    let mut file = File::open(file_name)?;

    println!("skip {} handshake bytes!", handshake_len);

    file.seek(SeekFrom::Start(handshake_len))?;

    let out_file = args.output;

    static FLV_HEADER: &[u8] = &[
        0x46, 0x4c, 0x56, 0x1, 0x05, 0x0, 0x0, 0x0, 0x9, 0x0, 0x0, 0x0, 0x0,
    ];

    let mut out_file = if let Some(out_file) = out_file {
        File::create(out_file).ok()
    } else {
        None
    };

    if let Some(ref mut out_file) = out_file {
        out_file.write_all(FLV_HEADER)?;
    }

    let mut deserializer = ChunkDeserializer::new();
    let mut message_number = 1;

    // only read one byte at a time to get a byte index for each message
    let mut buffer = [0; 4096];
    let mut current_index = 0;
    let mut last_message_end_index = 0;

    loop {
        let bytes_read = file.read(&mut buffer).unwrap();
        if bytes_read == 0 {
            println!("Finished reading log file!");
            return Ok(());
        }
        current_index += bytes_read;

        let mut has_read_one_payload = false;
        loop {
            let bytes = if has_read_one_payload {
                &[0_u8; 0]
            } else {
                &buffer[..bytes_read]
            };

            let payload = match deserializer.get_next_message(bytes).unwrap() {
                Some(payload) => payload,
                None => break,
            };

            println!(
                "Message: {}   Timestamp: {}   Type: {}    Stream_Id: {}   index: {} ({:x})",
                message_number,
                payload.timestamp.value,
                payload.type_id,
                payload.message_stream_id,
                last_message_end_index,
                last_message_end_index
            );

            if let Ok(message) = payload.to_rtmp_message() {
                match message {
                    RtmpMessage::Unknown {type_id, data}
                        => {
                        print!("Unknown {{ type_id: {}, data: ", type_id);
                        for x in 0..data.len() {
                            if x > 100 {
                                print!(".. ({}) ", data.len());
                                break;
                            }

                            print!("{:02x}", data[x]);
                        }
                        println!("}}");
                    },

                    RtmpMessage::Abort {stream_id}
                        => println!("Abort {{ stream_id: {} }}", stream_id),

                    RtmpMessage::Acknowledgement { sequence_number }
                        => println!("Acknowledgement {{ sequence_number: {} }}", sequence_number),

                    RtmpMessage::Amf0Command { command_name, transaction_id, command_object, additional_arguments }
                        => println!("Amf0Command {{ command_name: {}, transaction_id: {}, command_object: {:?}, additional_arguments: {:?} }}",
                                   command_name, transaction_id, command_object, additional_arguments),

                    RtmpMessage::Amf0Data { values }
                        => println!("RtmpMessage::Amf0Data {{ values: {:?} }}", values),

                    RtmpMessage::AudioData { data }
                        => {
                        print!("AudioData: {{ data: ");
                        for x in 0..data.len() {
                            if x > 100 {
                                print!(".. ({}) ", data.len());
                                break;
                            }

                            print!("{:02x}", data[x]);
                        }
                        println!("}}", );

                        if let Some(ref mut out_file) = out_file {
                            println!("write audio tag");
                            write_av_data(false, &payload, data.as_ref(), out_file)?;
                        }
                    },

                    RtmpMessage::SetChunkSize { size }
                        => {
                        deserializer.set_max_chunk_size(size as usize).unwrap();
                        println!("SetChunkSize {{ size: {} }}", size)
                    },

                    RtmpMessage::SetPeerBandwidth { size, limit_type }
                        => println!("SetPeerBandwidth {{ size: {}, limit_type: {:?} }}", size, limit_type),

                    RtmpMessage::UserControl { event_type, stream_id, buffer_length, timestamp }
                        => println!("UserControl {{ event_type: {:?}, stream_id: {:?}, buffer_length: {:?}, timestamp: {:?} }}",
                                    event_type, stream_id, buffer_length, timestamp),

                    RtmpMessage::VideoData { data }
                        => {
                        print!("VideoData: {{ data: ");
                        for x in 0..data.len() {
                            if x > 100 {
                                print!(".. ({}) ", data.len());
                                break;
                            }

                            print!("{:02x}", data[x]);
                        }
                        println!("}}");

                        if let Some(ref mut out_file) = out_file {
                            println!("write video tag");
                            write_av_data(true, &payload, data.as_ref(), out_file)?;
                        }
                    },

                    RtmpMessage::WindowAcknowledgement { size }
                        => println!("WindowAcknowledgement {{ size: {} }}", size),
                }
            } else {
                println!("Warning ------------ to rtmp message error, continue");
            }

            println!();

            message_number += 1;
            has_read_one_payload = true;
            last_message_end_index = current_index;
        }
    }
}
