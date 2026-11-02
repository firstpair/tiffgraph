(* dump.exe FILE...: what every grammar makes of each file, in the same text
   form as the Rust build's `cargo run --example dump`, so test/cross.py can
   compare the two builds byte for byte. *)
let max_xmp = 1 lsl 20

let role = function
  | Tiffgraph.Image_bytes -> "image" | Standard_value -> "standard"
  | Private_value -> "private" | Structure -> "structure"

let () =
  for i = 1 to Array.length Sys.argv - 1 do
    let path = Sys.argv.(i) in
    let bytes = try In_channel.with_open_bin path In_channel.input_all with Sys_error _ -> "" in
    let reader () = { Tiffgraph.size = Int64.of_int (String.length bytes);
                      read = (fun off n -> String.sub bytes (Int64.to_int off) n);
                      check = (fun () -> ()) } in
    Printf.printf "file\t%s\n" path;
    List.iter (fun (name, policy) ->
      match Tiffgraph.walk (policy ~max_xmp) (reader ()) with
      | Ok (Some { Tiffgraph.offset; length }) -> Printf.printf "%s\tok\t%Ld\t%d\n" name offset length
      | Ok None -> Printf.printf "%s\tnone\n" name
      | Error r -> Printf.printf "%s\trefused\t%s\n" name r
      | exception Invalid_argument _ -> Printf.printf "%s\treader\n" name)
      ["v0", Tiffgraph.v0; "v1", Tiffgraph.v1; "merge_only", Tiffgraph.merge_only; "layout", Tiffgraph.layout];
    (match Tiffgraph.unknown_fields ~max_xmp (reader ()) with
     | fields, stop ->
         List.iter (fun (d, tag, typ, n) -> Printf.printf "unknown\t%s\t%d\t%d\t%Ld\n" d tag typ n) fields;
         Printf.printf "unknown-stop\t%s\n" (Option.value stop ~default:"")
     | exception Invalid_argument _ -> print_string "unknown-stop\treader\n");
    (match Tiffgraph.component_regions ~max_xmp (reader ()) with
     | Ok regions ->
         List.iter (fun (r : Tiffgraph.component_region) ->
           Printf.printf "region\t%s\t%s\t%s\t%Ld\t%Ld\n" r.logical_id r.schema (role r.role) r.offset r.length) regions;
         print_string "regions\tok\n"
     | Error r -> Printf.printf "regions\trefused\t%s\n" r
     | exception Invalid_argument _ -> print_string "regions\treader\n")
  done
