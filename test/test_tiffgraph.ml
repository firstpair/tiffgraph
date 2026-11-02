(* Synthetic controls for Tiffgraph: every file is built here, in memory,
   from TIFF entries; no photo, catalog or external fixture is read. *)
module T = Tiffgraph

(* ---- A small TIFF writer ---------------------------------------------- *)

type value =
  | Data of int * int * string   (* type, count, encoded bytes *)
  | Child of entry list          (* LONG (or LONG8) offset to a child IFD *)
  | Strip_offset                 (* LONG offset of the strip data *)
  | Long_at of int64             (* LONG with a fixed value, e.g. a bad pointer *)
and entry = int * value

type file = { bytes : string; xmp_at : int64 option; strip_at : int64 }

let encode ~little width v =
  String.init width (fun i ->
    let shift = 8 * (if little then i else width - 1 - i) in
    Char.chr (Int64.to_int (Int64.logand (Int64.shift_right_logical v shift) 0xffL)))

let build ?(big = false) ?(little = true) ?(strip = String.make 32 '\xaa') ?(trailer = "") ifd0 =
  let b = Buffer.create 4096 and patches = ref [] in
  let w width v = Buffer.add_string b (encode ~little width v) in
  let pos () = Int64.of_int (Buffer.length b) in
  let patch at width v = patches := (at, encode ~little width v) :: !patches in
  let count_width, number_width, inline = if big then 8, 8, 8 else 2, 4, 4 in
  Buffer.add_string b (if little then "II" else "MM");
  if big then (w 2 43L; w 2 8L; w 2 0L) else w 2 42L;
  let first = pos () in
  w number_width 0L;
  let strip_patches = ref [] and xmp_at = ref None in
  let rec ifd entries =
    let at = pos () in
    w count_width (Int64.of_int (List.length entries));
    let slots = List.map (fun (tag, value) ->
      let typ, count = match value with
        | Data (typ, count, _) -> typ, count
        | Child _ | Strip_offset | Long_at _ -> 4, 1 in
      w 2 (Int64.of_int tag); w 2 (Int64.of_int typ); w number_width (Int64.of_int count);
      let slot = pos () in
      Buffer.add_string b (String.make inline '\000');
      (tag, value, slot)) entries in
    w number_width 0L;
    List.iter (fun (tag, value, slot) -> match value with
      | Data (_, _, data) when String.length data <= inline ->
          patches := (slot, data) :: !patches
      | Data (_, _, data) ->
          patch slot number_width (pos ());
          if tag = 700 && !xmp_at = None then xmp_at := Some (pos ());
          Buffer.add_string b data
      | Strip_offset -> strip_patches := slot :: !strip_patches
      | Long_at v -> patch slot 4 v
      | Child _ -> ()) slots;
    List.iter (fun (_, value, slot) -> match value with
      | Child children -> patch slot 4 (ifd children)
      | _ -> ()) slots;
    at in
  patch first number_width (ifd ifd0);
  let strip_at = pos () in
  Buffer.add_string b strip;
  Buffer.add_string b trailer;
  List.iter (fun slot -> patch slot 4 strip_at) !strip_patches;
  let bytes = Buffer.to_bytes b in
  List.iter (fun (at, s) -> Bytes.blit_string s 0 bytes (Int64.to_int at) (String.length s)) !patches;
  { bytes = Bytes.to_string bytes; xmp_at = !xmp_at; strip_at }

(* Entry helpers; little-endian-independent because single bytes or encoded later. *)
let bytes_ typ s = Data (typ, String.length s, s)
let short ?(little = true) v = Data (3, 1, encode ~little 2 (Int64.of_int v))
let long ?(little = true) v = Data (4, 1, encode ~little 4 (Int64.of_int v))
let xmp ?(typ = 1) n = (700, Data (typ, n, String.init n (fun i -> "<x:xmpmeta/>".[i mod 12])))
let dng_version = (50706, bytes_ 1 "\001\004\000\000")

let dng ?(little = true) ?(extra = []) ?(xmp_length = 100) ?(with_xmp = true) () =
  [254, long ~little 0; 256, short ~little 8; 257, short ~little 4;
   273, Strip_offset; 279, long ~little 32; dng_version]
  @ (if with_xmp then [xmp xmp_length] else []) @ extra

(* ---- Readers ---------------------------------------------------------- *)

let checks = ref 0
let reader s = { T.size = Int64.of_int (String.length s);
                 read = (fun off n ->
                   if off < 0L || Int64.add off (Int64.of_int n) > Int64.of_int (String.length s)
                   then failwith "read outside file";
                   String.sub s (Int64.to_int off) n);
                 check = (fun () -> incr checks) }

let max_xmp = 65536
let walk ?unknown policy f = T.walk ?unknown (policy ~max_xmp) (reader f.bytes)

(* ---- Assertions ------------------------------------------------------- *)

let failures = ref 0 and passed = ref 0
let case name ok = if ok then incr passed else (incr failures; Printf.printf "FAIL %s\n%!" name)
let contains s sub =
  let n = String.length sub in
  let rec go i = i + n <= String.length s && (String.sub s i n = sub || go (i + 1)) in go 0
let rejects name policy f reason =
  match walk policy f with
  | Error why when contains why reason -> case name true
  | Error why -> Printf.printf "  %s: got %S\n" name why; case name false
  | Ok _ -> Printf.printf "  %s: admitted\n" name; case name false
let finds name policy f =
  match walk policy f with
  | Ok (Some { T.offset; length }) -> case name (Some offset = f.xmp_at && length > 0)
  | Ok None -> Printf.printf "  %s: no packet\n" name; case name false
  | Error why -> Printf.printf "  %s: %s\n" name why; case name false

(* ---- Controls --------------------------------------------------------- *)

let () =
  let plain = build (dng ()) in
  List.iter (fun (label, policy) -> finds ("minimal DNG, " ^ label) policy plain)
    ["v0", T.v0; "v1", T.v1; "merge_only", T.merge_only; "layout", T.layout];
  (match walk T.v0 plain with
   | Ok (Some { T.length; _ }) -> case "packet length is the XMP count" (length = 100)
   | _ -> case "packet length is the XMP count" false);
  finds "big-endian DNG" T.v0 (build ~little:false (dng ~little:false ()));

  (* check: once after success, never after a refusal; reader errors propagate. *)
  checks := 0; ignore (walk T.v0 plain);
  case "check runs once after a successful walk" (!checks = 1);
  checks := 0; ignore (walk T.v0 (build (dng ~with_xmp:false ())));
  case "check does not run after a refusal" (!checks = 0);
  case "reader exceptions propagate unchanged"
    (match T.walk (T.v0 ~max_xmp) { (reader plain.bytes) with read = (fun _ _ -> raise Exit) } with
     | exception Exit -> true | _ -> false);

  rejects "not TIFF" T.v0 { plain with bytes = "PK" ^ String.sub plain.bytes 2 (String.length plain.bytes - 2) } "not TIFF";
  rejects "no XMP" T.v0 (build (dng ~with_xmp:false ())) "no embedded XMP";
  case "no XMP is fine when not required"
    (walk (fun ~max_xmp -> { (T.v1 ~max_xmp) with require_xmp = false }) (build (dng ~with_xmp:false ())) = Ok None);
  rejects "missing DNGVersion" T.v0
    (build (List.filter (fun (tag, _) -> tag <> 50706) (dng ()))) "missing DNGVersion";
  rejects "DNG version 2" T.v0
    (build (List.map (fun (tag, v) -> if tag = 50706 then (tag, bytes_ 1 "\002\000\000\000") else (tag, v)) (dng ())))
    "unsupported DNG version";
  rejects "duplicate tag" T.v0 (build (dng ~extra:[256, short 9] ())) "duplicate TIFF tag";
  rejects "cyclic Exif pointer" T.v0 (build (dng ~extra:[34665, Long_at 8L] ())) "repeated or cyclic IFD";
  rejects "unpaired strips" T.v0
    (build (List.filter (fun (tag, _) -> tag <> 279) (dng ()))) "unpaired image extents";
  rejects "multiple XMP packets" T.v0
    (build (dng ~extra:[330, Child [254, long 1; xmp 50]] ())) "multiple XMP packets";

  (* Extents: a value past the end of the file, a strip inside the packet. *)
  let truncated = build (dng ()) in
  rejects "value outside file" T.v0
    { truncated with bytes = String.sub truncated.bytes 0 (Option.get truncated.xmp_at |> Int64.to_int |> ( + ) 50) }
    "outside file";
  let probe = build (dng ()) in
  let aliased = build (List.map (fun (tag, v) ->
    if tag = 273 then (tag, Long_at (Option.get probe.xmp_at)) else (tag, v)) (dng ())) in
  rejects "strip inside the XMP packet" T.v0 aliased "XMP aliases protected extent";

  (* XMP shape. *)
  rejects "XMP over max_xmp" T.v0 (build (dng ~xmp_length:(max_xmp + 1) ())) "unsupported XMP type or packet size";
  let undefined = build (List.map (fun (tag, v) -> if tag = 700 then xmp ~typ:7 100 else (tag, v)) (dng ())) in
  rejects "UNDEFINED XMP under v1" T.v1 undefined "unsupported XMP type";
  finds "UNDEFINED XMP under merge_only" T.merge_only undefined;

  (* BigTIFF is v1 and later. *)
  let big = build ~big:true (dng ()) in
  rejects "BigTIFF under v0" T.v0 big "unsupported TIFF version";
  finds "BigTIFF under v1" T.v1 big;
  finds "big-endian BigTIFF under v1" T.v1 (build ~big:true ~little:false (dng ~little:false ()));

  (* Grammar widening: unknown fields, simple types, admissions, MakerNote. *)
  let unknown_long = build (dng ~extra:[65001, long 7] ()) in
  rejects "unknown LONG under v1" T.v1 unknown_long "unsupported TIFF tag 65001";
  rejects "unknown LONG under merge_only" T.merge_only unknown_long "unsupported TIFF tag 65001";
  finds "unknown LONG under layout" T.layout unknown_long;
  let unknown_short = build (dng ~extra:[65002, short 7] ()) in
  rejects "unknown SHORT under v1" T.v1 unknown_short "unsupported TIFF tag 65002";
  finds "unknown SHORT (simple type) under merge_only" T.merge_only unknown_short;
  (match T.unknown_fields ~max_xmp (reader (build (dng ~extra:[65001, long 7; 65003, bytes_ 7 "abcdefgh"] ())).bytes) with
   | fields, None -> case "unknown_fields lists every unknown field"
                       (fields = ["Image", 65001, 4, 1L; "Image", 65003, 7, 8L])
   | _, Some why -> Printf.printf "  unknown_fields stopped: %s\n" why; case "unknown_fields lists every unknown field" false);
  T.admit_fields ["Image", 65001, 4];
  finds "admitted field under merge_only" T.merge_only unknown_long;
  rejects "admission is merge_only only" T.v1 unknown_long "unsupported TIFF tag 65001";
  T.admit_fields [];
  rejects "admissions can be withdrawn" T.merge_only unknown_long "unsupported TIFF tag 65001";
  case "parse_admission accepts Exif:37500:7" (T.parse_admission "Exif:37500:7" = Ok ("Exif", 37500, 7));
  case "parse_admission refuses IFD type" (Result.is_error (T.parse_admission "Image:65001:13"));
  case "parse_admission refuses CameraProfile" (Result.is_error (T.parse_admission "CameraProfile:50708:2"));
  let maker = build (dng ~extra:[34665, Child [37500, bytes_ 7 "MakerNote-bytes"]] ()) in
  rejects "MakerNote under v1" T.v1 maker "unsupported TIFF tag 37500";
  finds "MakerNote under merge_only" T.merge_only maker;
  let private_data = build (dng ~extra:[330, Child [254, long 1; 50740, bytes_ 1 "private!"]] ()) in
  rejects "DNGPrivateData outside the primary IFD" T.merge_only private_data "DNGPrivateData outside primary IFD";
  finds "DNGPrivateData in the primary IFD" T.merge_only (build (dng ~extra:[50740, bytes_ 1 "private!"] ()));

  (* Component regions. *)
  (match T.component_regions ~max_xmp (reader (build (dng ~with_xmp:false ~extra:[65001, long 7] ())).bytes) with
   | Error why -> Printf.printf "  component_regions: %s\n" why; case "component regions" false
   | Ok regions ->
       let f = build (dng ~with_xmp:false ~extra:[65001, long 7] ()) in
       let has p = List.exists p regions in
       case "header is structure" (has (fun r -> r.T.logical_id = "header" && r.role = T.Structure && r.offset = 0L && r.length = 8L));
       case "strip is image bytes" (has (fun r -> r.T.role = T.Image_bytes && r.offset = f.strip_at && r.length = 32L));
       case "unknown field is a private value" (has (fun r -> r.T.logical_id = "ifd/0/tag/65001/value" && r.role = T.Private_value));
       case "standard field is a standard value" (has (fun r -> r.T.logical_id = "ifd/0/tag/256/value" && r.role = T.Standard_value));
       case "regions lie inside the file"
         (List.for_all (fun r -> r.T.offset >= 0L && Int64.add r.offset r.length <= Int64.of_int (String.length f.bytes)) regions));

  Printf.printf "%s %d passed, %d failed\n" (if !failures = 0 then "ok" else "FAIL") !passed !failures;
  if !failures > 0 then exit 1
