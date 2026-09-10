# cargo-mutants 27.1.0 receipt checks. Unviable is never counted as caught.
def demand($condition; $message): if $condition then . else error($message) end;
def failed_status: type=="object" and (.Failure|type=="number" and .!=0);
def phase($name): first(.phase_results[] | select(.phase==$name));
def command:
    {packages:([.[]|select(startswith("--package="))|sub("@0\\.0\\.0$";"")]|unique),
     arguments:[.[]|select(startswith("--package=")|not)]};
def valid_phase:
    (.phase|.=="Build" or .=="Test") and (.duration|type=="number" and .>=0)
    and (.argv|type=="array" and length>0 and all(type=="string"));
def phases($names):
    (.phase_results|type=="array") and ([.phase_results[].phase]|sort)==($names|sort)
    and all(.phase_results[];valid_phase);
demand(type=="array" and length==1;"exactly one inventory document required")
| .[0] as $inventory | $inventory
| demand(type=="array";"inventory must be an array")
| demand(all(.[];type=="object" and .file==$source and (.name|type=="string" and startswith($source+":")));"wrong scope or malformed mutant identity")
| demand((map(.name)|unique|length)==length;"duplicate selected mutant")
| demand($inventory_hash|test("^[0-9a-f]{64}$");"invalid inventory digest")
| if length==0 then
    demand($kind!="full";"required exhaustive scope selected zero mutants")
    | demand($terminal_present==false and ($terminal|length)==0;"empty selection has unexpected terminal evidence")
    | {scope:$scope,status:"not-required",selected:0,executed:0,caught:0,unviable:0,baseline:false,
       inventory_sha256:$inventory_hash,outcomes_sha256:null,reason:"successful empty selection"}
  else
    demand($terminal_present and ($terminal|length)==1 and ($terminal_hash|test("^[0-9a-f]{64}$"));"missing terminal outcome report or digest")
    | $terminal[0] as $report
    | demand($report.cargo_mutants_version=="27.1.0";"wrong mutation tool version")
    | demand(($report.outcomes|type)=="array";"missing outcomes")
    | [$report.outcomes[]|select(.scenario=="Baseline")] as $baselines
    | [$report.outcomes[]|select(.scenario!="Baseline")] as $mutants
    | demand(($baselines|length)==1;"exactly one baseline required")
    | $baselines[0] as $baseline
    | demand($baseline.summary=="Success" and ($baseline|phases(["Build","Test"]))
        and all($baseline.phase_results[];.process_status=="Success");"unsuccessful or incomplete baseline")
    | demand(($mutants|length)==($inventory|length)
        and ([$mutants[].scenario.Mutant.name]|sort)==($inventory|map(.name)|sort);"missing, duplicate or unexpected terminal mutant")
    | demand(all($mutants[];. as $result |
        ($inventory|map(select(.name==$result.scenario.Mutant.name))|first|del(.diff))==$result.scenario.Mutant
        and ($result|if .summary=="CaughtMutant" then
            phases(["Build","Test"]) and ((phase("Build")).process_status=="Success")
            and ((phase("Test")).process_status|failed_status)
          elif .summary=="Unviable" then
            phases(["Build"]) and ((phase("Build")).process_status|failed_status)
          else false end));"invalid terminal classification or mutant identity")
    | (if $scope=="native-platform-contract-mutants" or $scope=="native-platform-mutants"
          or $scope=="native-studio-accessibility-mutants" then ["--package=alpine-platform-macos","--package=alpine-studio"]
        elif $scope=="native-runtime-mutants" then ["--package=alpine-runtime","--package=alpine-studio"]
        else ["--package="+($source|split("/")[1])] end) as $packages
    | demand(all($baseline.phase_results[];(.argv|command|.packages)==($packages|sort));"baseline omits required package scope")
    | demand(all($mutants[];. as $result | all(.phase_results[];. as $phase |
        ($phase.argv|command)==($baseline|phase($phase.phase)|.argv|command)));"baseline and mutant commands differ")
    | ([$mutants[]|select(.summary=="CaughtMutant")]|length) as $caught
    | ([$mutants[]|select(.summary=="Unviable")]|length) as $unviable
    | demand($report.total_mutants==($inventory|length) and $report.caught==$caught
        and $report.unviable==$unviable and $report.missed==0 and $report.timeout==0
        and $report.success==0;"terminal counters disagree")
    | demand($kind!="strict" or $unviable==0;"runtime scope cannot accept unviable mutants")
    | {scope:$scope,status:"complete",selected:($inventory|length),executed:($mutants|length),
       caught:$caught,unviable:$unviable,baseline:true,inventory_sha256:$inventory_hash,
       outcomes_sha256:$terminal_hash,
       build_seconds:([$report.outcomes[].phase_results[]|select(.phase=="Build")|.duration]|add),
       test_seconds:([$report.outcomes[].phase_results[]|select(.phase=="Test")|.duration]|add)}
  end
