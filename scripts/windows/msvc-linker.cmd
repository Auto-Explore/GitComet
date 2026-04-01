@echo off
setlocal EnableExtensions

set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" (
  echo gitcomet: could not find vswhere.exe at "%VSWHERE%".
  echo gitcomet: install Visual Studio 2022 Build Tools or Community with C++ workload.
  exit /b 1
)

set "VSINSTALL="
for /f "usebackq delims=" %%I in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do (
  set "VSINSTALL=%%I"
)
if not defined VSINSTALL (
  echo gitcomet: could not locate a Visual Studio installation with MSVC tools.
  echo gitcomet: install the "Desktop development with C++" workload.
  exit /b 1
)

set "MSVC_TOOLS=%VSINSTALL%\VC\Tools\MSVC"
if not exist "%MSVC_TOOLS%" (
  echo gitcomet: MSVC tools directory not found: "%MSVC_TOOLS%".
  exit /b 1
)

set "MSVC_VER="
for /f "delims=" %%I in ('dir /b /ad "%MSVC_TOOLS%" ^| sort /r') do (
  set "MSVC_VER=%%I"
  goto :msvc_version_found
)
:msvc_version_found
if not defined MSVC_VER (
  echo gitcomet: no MSVC toolset found under "%MSVC_TOOLS%".
  exit /b 1
)

set "MSVC_ROOT=%MSVC_TOOLS%\%MSVC_VER%"
set "LINK_EXE=%MSVC_ROOT%\bin\Hostx64\x64\link.exe"
if not exist "%LINK_EXE%" (
  echo gitcomet: link.exe not found at "%LINK_EXE%".
  exit /b 1
)

set "KITS_ROOT=%ProgramFiles(x86)%\Windows Kits\10"
set "KITS_LIB=%KITS_ROOT%\Lib"
set "KITS_INC=%KITS_ROOT%\Include"

if not exist "%KITS_LIB%" (
  echo gitcomet: Windows SDK not found at "%KITS_LIB%".
  echo gitcomet: install the Windows 10 or Windows 11 SDK component in Visual Studio Installer.
  exit /b 1
)

set "SDK_VER="
for /f "delims=" %%I in ('dir /b /ad "%KITS_LIB%" ^| sort /r') do (
  if exist "%KITS_LIB%\%%I\um\x64\kernel32.lib" (
    set "SDK_VER=%%I"
    goto :sdk_found
  )
)
:sdk_found
if not defined SDK_VER (
  echo gitcomet: Windows SDK libraries missing ^(kernel32.lib not found^).
  echo gitcomet: install the Windows 10 or Windows 11 SDK component.
  exit /b 1
)

set "MSVC_LIB=%MSVC_ROOT%\lib\x64"
set "MSVC_INCLUDE=%MSVC_ROOT%\include"
set "SDK_UM_LIB=%KITS_LIB%\%SDK_VER%\um\x64"
set "SDK_UCRT_LIB=%KITS_LIB%\%SDK_VER%\ucrt\x64"
set "SDK_SHARED_INC=%KITS_INC%\%SDK_VER%\shared"
set "SDK_UM_INC=%KITS_INC%\%SDK_VER%\um"
set "SDK_UCRT_INC=%KITS_INC%\%SDK_VER%\ucrt"
set "SDK_WINRT_INC=%KITS_INC%\%SDK_VER%\winrt"
set "SDK_CPPWINRT_INC=%KITS_INC%\%SDK_VER%\cppwinrt"

set "LIB=%MSVC_LIB%;%SDK_UM_LIB%;%SDK_UCRT_LIB%;%LIB%"
set "LIBPATH=%MSVC_LIB%;%SDK_UM_LIB%;%SDK_UCRT_LIB%;%LIBPATH%"
set "INCLUDE=%MSVC_INCLUDE%;%SDK_SHARED_INC%;%SDK_UM_INC%;%SDK_UCRT_INC%;%SDK_WINRT_INC%;%SDK_CPPWINRT_INC%;%INCLUDE%"

rem GitComet's GPUI diff/render paths are substantially deeper in debug builds.
rem The Windows default 1 MiB main-thread stack is not enough there, which can
rem abort the process with a stack overflow before Rust's panic hook runs.
if "%GITCOMET_LINK_STACK_RESERVE%"=="" set "GITCOMET_LINK_STACK_RESERVE=8388608"

"%LINK_EXE%" /STACK:%GITCOMET_LINK_STACK_RESERVE% %*
set "EXITCODE=%ERRORLEVEL%"
exit /b %EXITCODE%
