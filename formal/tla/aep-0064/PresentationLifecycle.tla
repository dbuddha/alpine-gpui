------------------------- MODULE PresentationLifecycle -------------------------
EXTENDS Naturals, TLC

CONSTANTS MaxRevision, MaxEpoch, MaxEnvironmentChanges, FaultyStalePresent

VARIABLES app, link, visible, sized, dirty, requestedRevision,
          presentedRevision, surfaceEpoch, environmentChanges, phase,
          frameRevision, frameEpoch, resource, attemptSubmits, presentCalls,
          eligibleAtSubmit, outcome

vars == <<app, link, visible, sized, dirty, requestedRevision,
          presentedRevision, surfaceEpoch, environmentChanges, phase,
          frameRevision, frameEpoch, resource, attemptSubmits, presentCalls,
          eligibleAtSubmit, outcome>>

AppStates == {"Running", "Stopping", "Stopped"}
LinkStates == {"Paused", "Running", "Invalid"}
Phases == {"Idle", "Prepared", "Encoding", "Submitted", "PresentCalled"}
Resources == {"Free", "Drawable", "InFlight"}
Outcomes == {"None", "Presented", "Dropped", "Failed"}
TerminalOutcomes == {"Presented", "Dropped", "Failed"}

Eligible ==
    /\ app = "Running"
    /\ visible
    /\ sized
    /\ dirty
    /\ frameRevision = requestedRevision
    /\ frameEpoch = surfaceEpoch

TypeOK ==
    /\ app \in AppStates
    /\ link \in LinkStates
    /\ visible \in BOOLEAN
    /\ sized \in BOOLEAN
    /\ dirty \in BOOLEAN
    /\ requestedRevision \in 0..MaxRevision
    /\ presentedRevision \in 0..MaxRevision
    /\ surfaceEpoch \in 0..MaxEpoch
    /\ environmentChanges \in 0..MaxEnvironmentChanges
    /\ phase \in Phases
    /\ frameRevision \in 0..MaxRevision
    /\ frameEpoch \in 0..MaxEpoch
    /\ resource \in Resources
    /\ attemptSubmits \in 0..1
    /\ presentCalls \in 0..1
    /\ eligibleAtSubmit \in BOOLEAN
    /\ outcome \in Outcomes

Init ==
    /\ app = "Running"
    /\ link = "Paused"
    /\ visible = TRUE
    /\ sized = TRUE
    /\ dirty = FALSE
    /\ requestedRevision = 0
    /\ presentedRevision = 0
    /\ surfaceEpoch = 0
    /\ environmentChanges = 0
    /\ phase = "Idle"
    /\ frameRevision = 0
    /\ frameEpoch = 0
    /\ resource = "Free"
    /\ attemptSubmits = 0
    /\ presentCalls = 0
    /\ eligibleAtSubmit = FALSE
    /\ outcome = "None"

Invalidate ==
    /\ app = "Running"
    /\ requestedRevision < MaxRevision
    /\ requestedRevision' = requestedRevision + 1
    /\ dirty' = TRUE
    /\ outcome' = "None"
    /\ UNCHANGED <<app, link, visible, sized, presentedRevision,
                    surfaceEpoch, environmentChanges, phase, frameRevision,
                    frameEpoch, resource, attemptSubmits, presentCalls,
                    eligibleAtSubmit>>

AdvanceSurfaceEpoch ==
    /\ app = "Running"
    /\ surfaceEpoch < MaxEpoch
    /\ environmentChanges < MaxEnvironmentChanges
    /\ surfaceEpoch' = surfaceEpoch + 1
    /\ environmentChanges' = environmentChanges + 1
    /\ dirty' = TRUE
    /\ outcome' = "None"
    /\ UNCHANGED <<app, link, visible, sized, requestedRevision,
                    presentedRevision, phase, frameRevision, frameEpoch,
                    resource, attemptSubmits, presentCalls, eligibleAtSubmit>>

ToggleVisibility ==
    /\ app = "Running"
    /\ environmentChanges < MaxEnvironmentChanges
    /\ visible' = ~visible
    /\ environmentChanges' = environmentChanges + 1
    /\ link' = IF visible' THEN link ELSE "Paused"
    /\ UNCHANGED <<app, sized, dirty, requestedRevision, presentedRevision,
                    surfaceEpoch, phase, frameRevision, frameEpoch, resource,
                    attemptSubmits, presentCalls, eligibleAtSubmit, outcome>>

ToggleSize ==
    /\ app = "Running"
    /\ environmentChanges < MaxEnvironmentChanges
    /\ sized' = ~sized
    /\ environmentChanges' = environmentChanges + 1
    /\ link' = IF sized' THEN link ELSE "Paused"
    /\ UNCHANGED <<app, visible, dirty, requestedRevision, presentedRevision,
                    surfaceEpoch, phase, frameRevision, frameEpoch, resource,
                    attemptSubmits, presentCalls, eligibleAtSubmit, outcome>>

Resume ==
    /\ app = "Running"
    /\ link = "Paused"
    /\ visible
    /\ sized
    /\ dirty
    /\ phase = "Idle"
    /\ resource = "Free"
    /\ link' = "Running"
    /\ UNCHANGED <<app, visible, sized, dirty, requestedRevision,
                    presentedRevision, surfaceEpoch, environmentChanges, phase,
                    frameRevision, frameEpoch, resource, attemptSubmits,
                    presentCalls,
                    eligibleAtSubmit, outcome>>

Prepare ==
    /\ app = "Running"
    /\ link = "Running"
    /\ visible
    /\ sized
    /\ dirty
    /\ phase = "Idle"
    /\ resource = "Free"
    /\ phase' = "Prepared"
    /\ frameRevision' = requestedRevision
    /\ frameEpoch' = surfaceEpoch
    /\ attemptSubmits' = 0
    /\ presentCalls' = 0
    /\ eligibleAtSubmit' = FALSE
    /\ UNCHANGED <<app, link, visible, sized, dirty, requestedRevision,
                    presentedRevision, surfaceEpoch, environmentChanges,
                    resource, outcome>>

DropPreparedStale ==
    /\ app = "Running"
    /\ phase = "Prepared"
    /\ ~Eligible
    /\ phase' = "Idle"
    /\ resource' = "Free"
    /\ outcome' = "Dropped"
    /\ link' = IF visible /\ sized /\ dirty THEN "Running" ELSE "Paused"
    /\ UNCHANGED <<app, visible, sized, dirty, requestedRevision,
                    presentedRevision, surfaceEpoch, environmentChanges,
                    frameRevision, frameEpoch, attemptSubmits, presentCalls,
                    eligibleAtSubmit>>

BeginUpdate ==
    /\ link = "Running"
    /\ phase = "Prepared"
    /\ resource = "Free"
    /\ Eligible
    /\ phase' = "Encoding"
    /\ resource' = "Drawable"
    /\ UNCHANGED <<app, link, visible, sized, dirty, requestedRevision,
                    presentedRevision, surfaceEpoch, environmentChanges,
                    frameRevision, frameEpoch, attemptSubmits, presentCalls,
                    eligibleAtSubmit, outcome>>

DropEncodingStale ==
    /\ phase = "Encoding"
    /\ resource = "Drawable"
    /\ ~Eligible
    /\ phase' = "Idle"
    /\ resource' = "Free"
    /\ outcome' = "Dropped"
    /\ link' = IF app = "Running" /\ visible /\ sized /\ dirty
                 THEN "Running" ELSE link
    /\ UNCHANGED <<app, visible, sized, dirty, requestedRevision,
                    presentedRevision, surfaceEpoch, environmentChanges,
                    frameRevision, frameEpoch, attemptSubmits, presentCalls,
                    eligibleAtSubmit>>

Submit ==
    /\ link = "Running"
    /\ phase = "Encoding"
    /\ resource = "Drawable"
    /\ Eligible
    /\ attemptSubmits = 0
    /\ phase' = "Submitted"
    /\ resource' = "InFlight"
    /\ attemptSubmits' = 1
    /\ eligibleAtSubmit' = TRUE
    /\ UNCHANGED <<app, link, visible, sized, dirty, requestedRevision,
                    presentedRevision, surfaceEpoch, environmentChanges,
                    frameRevision, frameEpoch, presentCalls, outcome>>

CallPresent ==
    /\ phase = "Submitted"
    /\ resource = "InFlight"
    /\ attemptSubmits = 1
    /\ presentCalls = 0
    /\ phase' = "PresentCalled"
    /\ presentCalls' = 1
    /\ UNCHANGED <<app, link, visible, sized, dirty, requestedRevision,
                    presentedRevision, surfaceEpoch, environmentChanges,
                    frameRevision, frameEpoch, resource, attemptSubmits,
                    eligibleAtSubmit, outcome>>

CompletePresentation ==
    /\ phase = "PresentCalled"
    /\ resource = "InFlight"
    /\ presentCalls = 1
    /\ phase' = "Idle"
    /\ resource' = "Free"
    /\ outcome' = IF Eligible THEN "Presented" ELSE "Dropped"
    /\ presentedRevision' = IF Eligible THEN frameRevision
                             ELSE presentedRevision
    /\ dirty' = IF Eligible THEN FALSE ELSE dirty
    /\ link' = IF app = "Running" /\ ~Eligible /\ visible /\ sized /\ dirty
                 THEN "Running"
                 ELSE IF app = "Running" THEN "Paused" ELSE "Invalid"
    /\ UNCHANGED <<app, visible, sized, requestedRevision, surfaceEpoch,
                    environmentChanges, frameRevision, frameEpoch,
                    attemptSubmits, presentCalls, eligibleAtSubmit>>

FailActive ==
    /\ phase \in {"Encoding", "Submitted", "PresentCalled"}
    /\ resource \in {"Drawable", "InFlight"}
    /\ phase' = "Idle"
    /\ resource' = "Free"
    /\ dirty' = FALSE
    /\ outcome' = "Failed"
    /\ link' = IF app = "Running" THEN "Paused" ELSE "Invalid"
    /\ UNCHANGED <<app, visible, sized, requestedRevision,
                    presentedRevision, surfaceEpoch, environmentChanges,
                    frameRevision, frameEpoch, attemptSubmits, presentCalls,
                    eligibleAtSubmit>>

BeginShutdownIdle ==
    /\ app = "Running"
    /\ phase \in {"Idle", "Prepared"}
    /\ resource = "Free"
    /\ app' = "Stopping"
    /\ link' = "Invalid"
    /\ phase' = "Idle"
    /\ dirty' = FALSE
    /\ outcome' = IF phase = "Prepared" THEN "Dropped" ELSE outcome
    /\ UNCHANGED <<visible, sized, requestedRevision, presentedRevision,
                    surfaceEpoch, environmentChanges, frameRevision,
                    frameEpoch, resource, attemptSubmits, presentCalls,
                    eligibleAtSubmit>>

BeginShutdownEncoding ==
    /\ app = "Running"
    /\ phase = "Encoding"
    /\ resource = "Drawable"
    /\ app' = "Stopping"
    /\ link' = "Invalid"
    /\ phase' = "Idle"
    /\ resource' = "Free"
    /\ dirty' = FALSE
    /\ outcome' = "Dropped"
    /\ UNCHANGED <<visible, sized, requestedRevision, presentedRevision,
                    surfaceEpoch, environmentChanges, frameRevision,
                    frameEpoch, attemptSubmits, presentCalls, eligibleAtSubmit>>

BeginShutdownCommitted ==
    /\ app = "Running"
    /\ phase \in {"Submitted", "PresentCalled"}
    /\ resource = "InFlight"
    /\ app' = "Stopping"
    /\ link' = "Invalid"
    /\ dirty' = FALSE
    /\ UNCHANGED <<visible, sized, requestedRevision, presentedRevision,
                    surfaceEpoch, environmentChanges, phase, frameRevision,
                    frameEpoch, resource, attemptSubmits, presentCalls,
                    eligibleAtSubmit, outcome>>

StopAfterDrain ==
    /\ app = "Stopping"
    /\ phase = "Idle"
    /\ resource = "Free"
    /\ app' = "Stopped"
    /\ UNCHANGED <<link, visible, sized, dirty, requestedRevision,
                    presentedRevision, surfaceEpoch, environmentChanges, phase,
                    frameRevision, frameEpoch, resource, attemptSubmits,
                    presentCalls, eligibleAtSubmit, outcome>>

FaultyPresentStale ==
    /\ FaultyStalePresent
    /\ app = "Running"
    /\ phase = "PresentCalled"
    /\ resource = "InFlight"
    /\ presentCalls = 1
    /\ \/ frameRevision # requestedRevision
       \/ frameEpoch # surfaceEpoch
    /\ phase' = "Idle"
    /\ resource' = "Free"
    /\ dirty' = FALSE
    /\ presentedRevision' = frameRevision
    /\ outcome' = "Presented"
    /\ link' = "Paused"
    /\ UNCHANGED <<app, visible, sized, requestedRevision, surfaceEpoch,
                    environmentChanges, frameRevision, frameEpoch,
                    attemptSubmits, presentCalls, eligibleAtSubmit>>

Next ==
    \/ Invalidate
    \/ AdvanceSurfaceEpoch
    \/ ToggleVisibility
    \/ ToggleSize
    \/ Resume
    \/ Prepare
    \/ DropPreparedStale
    \/ BeginUpdate
    \/ DropEncodingStale
    \/ Submit
    \/ CallPresent
    \/ CompletePresentation
    \/ FailActive
    \/ BeginShutdownIdle
    \/ BeginShutdownEncoding
    \/ BeginShutdownCommitted
    \/ StopAfterDrain
    \/ FaultyPresentStale

Spec ==
    /\ Init
    /\ [][Next]_vars
    /\ WF_vars(Resume)
    /\ WF_vars(Prepare)
    /\ WF_vars(DropPreparedStale)
    /\ WF_vars(BeginUpdate)
    /\ WF_vars(DropEncodingStale)
    /\ WF_vars(Submit)
    /\ WF_vars(CallPresent)
    /\ WF_vars(CompletePresentation)
    /\ WF_vars(BeginShutdownIdle \/ BeginShutdownEncoding
               \/ BeginShutdownCommitted)
    /\ WF_vars(StopAfterDrain)

LinkOwnership ==
    /\ (app = "Running" => link \in {"Paused", "Running"})
    /\ (app \in {"Stopping", "Stopped"} => link = "Invalid")
    /\ (link = "Running" => app = "Running" /\ visible /\ sized /\ dirty)

ResourceMatchesPhase ==
    /\ (phase \in {"Idle", "Prepared"} => resource = "Free")
    /\ (phase = "Encoding" => resource = "Drawable")
    /\ (phase \in {"Submitted", "PresentCalled"} => resource = "InFlight")

SingleSubmissionPerAttempt == attemptSubmits <= 1

SinglePresentCallPerAttempt == presentCalls <= 1

CommittedWasEligible ==
    phase \in {"Submitted", "PresentCalled"} =>
        attemptSubmits = 1 /\ eligibleAtSubmit

PresentFollowsSubmission ==
    phase = "PresentCalled" => attemptSubmits = 1 /\ presentCalls = 1

PresentedIsCurrent ==
    outcome = "Presented" =>
        /\ presentedRevision = requestedRevision
        /\ presentedRevision = frameRevision
        /\ frameEpoch = surfaceEpoch
        /\ attemptSubmits = 1
        /\ presentCalls = 1
        /\ eligibleAtSubmit

CleanIdleIsPaused ==
    app = "Running" /\ phase = "Idle" /\ ~dirty => link = "Paused"

StoppedIsDrained ==
    app = "Stopped" =>
        /\ link = "Invalid"
        /\ phase = "Idle"
        /\ resource = "Free"

CommittedEventuallyTerminates ==
    [] (phase \in {"Submitted", "PresentCalled"} => <> (phase = "Idle"))

ShutdownEventuallyStops ==
    [] (app = "Stopping" => <> (app = "Stopped"))

VisibleDirtyEventuallySettles ==
    [] (app = "Running" /\ visible /\ sized /\ dirty
        => <> (~dirty \/ app # "Running" \/ outcome \in TerminalOutcomes))

CanReachTerminal == <> (outcome \in TerminalOutcomes \/ app = "Stopped")

=============================================================================
