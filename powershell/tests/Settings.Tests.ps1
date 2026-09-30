BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule
}

AfterAll {
    Set-TrexClock -SystemClock -TimeZoneOffset '00:00' -DateOrder DayFirst -Confirm:$false
}

Describe 'The clock' {
    It 'fixes now, and a typed predicate reads it' {
        Set-TrexClock -Now ([DateTimeOffset]::new(2026, 9, 27, 0, 0, 0, [TimeSpan]::Zero)) -Confirm:$false
        $clock = Get-TrexClock
        $clock.Fixed | Should -BeTrue
        $clock.Now.UtcDateTime | Should -Be ([datetime]::new(2026, 9, 27, 0, 0, 0, [DateTimeKind]::Utc))
        Test-TrexMatch '\T{age<24h}' 'at 2026-09-26T12:00:00Z' | Should -BeTrue
        Test-TrexMatch '\T{age<24h}' 'at 2026-09-20T12:00:00Z' | Should -BeFalse
    }

    It 'returns to the system clock' {
        Set-TrexClock -SystemClock -Confirm:$false
        (Get-TrexClock).Fixed | Should -BeFalse
    }

    It 'sets the zone offset and the date order' {
        $clock = Set-TrexClock -TimeZoneOffset '-04:00' -DateOrder MonthFirst -PassThru -Confirm:$false
        $clock.TimeZoneOffset | Should -Be ([TimeSpan]::FromHours(-4))
        $clock.DateOrder | Should -Be ([Trex.DateOrder]::MonthFirst)
    }
}

Describe 'Get-TrexInfo' {
    It 'writes the engine version' {
        (Get-TrexInfo).Version | Should -Match '^\d+\.\d+\.\d+'
    }
}
