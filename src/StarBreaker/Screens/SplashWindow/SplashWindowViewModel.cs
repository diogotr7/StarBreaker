using System.Collections.ObjectModel;
using System.Text.Json;
using System.Text.Json.Serialization;
using Avalonia.Platform.Storage;
using Avalonia.Threading;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Microsoft.Extensions.Logging;
using StarBreaker.Services;
using static System.Environment;

namespace StarBreaker.Screens;

public sealed partial class SplashWindowViewModel : ViewModelBase
{
    private readonly ILogger<SplashWindowViewModel> _logger;
    private readonly IP4kService _p4kService;

    [ObservableProperty] private ObservableCollection<StarCitizenInstallationViewModel> _installations;
    [ObservableProperty] private double? _progress;

    public SplashWindowViewModel(ILogger<SplashWindowViewModel> logger, IP4kService p4kService)
    {
        _logger = logger;
        _p4kService = p4kService;
        _installations = [];
        Progress = null;
        LoadDefaultP4kLocations();
    }

    public event EventHandler? P4kLoaded;

    private void OnP4kLoaded()
    {
        P4kLoaded?.Invoke(this, EventArgs.Empty);
    }

    private async Task LoadP4k(string path)
    {
        Progress = 0;
        var progress = new Progress<double>(x => Dispatcher.UIThread.Post(() => Progress = x));

        await Task.Run(() => _p4kService.OpenP4k(path, progress));

        OnP4kLoaded();
    }

    [RelayCommand]
    public async Task PickP4k()
    {
        _logger?.LogTrace("PickP4k enter");
        var defaultPath = await App.StorageProvider.TryGetFolderFromPathAsync($"{Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles)}{Constants.DefaultStarCitizenFolder}");
        var task = App.StorageProvider.OpenFilePickerAsync(Constants.GetP4kFilter(defaultPath));
        var file = await task;
        if (file.Count != 1)
        {
            _logger?.LogError("OpenFilePickerAsync returned {Count} files", file.Count);
            return;
        }

        _logger?.LogTrace("PickP4k exit: {Path}", file[0].Path);

        await LoadP4k(file[0].Path.LocalPath);
    }

    public void LoadDefaultP4kLocations()
    {
        var currentInstallDirectory = GetInstallDirectory();
        if (currentInstallDirectory.Length == 0)
        {
            GetP4ksFromDirectory($"{Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles)}{Constants.DefaultStarCitizenFolder}");
        }
        else
        {
            GetP4ksFromDirectory(currentInstallDirectory);
        }
    }

    private void GetP4ksFromDirectory(string installationPath)
    {
        if (!Directory.Exists(installationPath))
        {
            return;
        }
        var p4ks = Directory.GetFiles(installationPath, Constants.DataP4k, SearchOption.AllDirectories);
        if (p4ks.Length == 0)
        {
            _logger.LogError("No Data.p4k files found");
            return;
        }

        _logger.LogTrace("Found {Count} Data.p4k files", p4ks.Length);

        foreach (var p4k in p4ks)
        {
            var directoryName = Path.GetDirectoryName(p4k);
            if (directoryName is null)
            {
                _logger.LogError("Failed to get directory name for {Path}", p4k);
                continue;
            }
            
            BuildManifest? manifest = null;
            try
            {
                if (File.Exists(Path.Combine(directoryName, Constants.BuildManifest)))
                {
                    manifest = JsonSerializer.Deserialize(
                        File.ReadAllText(Path.Combine(directoryName, Constants.BuildManifest)),
                        StarBreakerSerializerContext.Default.BuildManifest
                    );
                }
            }
            catch
            {
                //fine to ignore
                
            }

            Installations.Add(new StarCitizenInstallationViewModel
            {
                ChannelName = new DirectoryInfo(directoryName).Name,
                Path = p4k,
                Manifest = manifest
            });
        }
    }

    [RelayCommand]
    public async Task ClickP4kLocation(string file)
    {
        _logger.LogTrace("ClickP4kLocation {Path}", file);
        await LoadP4k(file);
    }

    /// <summary>
    /// Checks the RSI Launcher logs for a Star Citizen Install Directory
    /// </summary>
    /// <returns>The current Star Citizen install directory</returns>
    private string GetInstallDirectory()
    {
        var launcherPath = $"{Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData)}{Constants.DefaultRSILauncherFolder}";
        if (!File.Exists($"{launcherPath}\\log.log"))
        {
            _logger.LogError("Failed to find RSI Launcher log");
            return String.Empty;
        }
        var lines = File.ReadLines($"{launcherPath}\\log.log");
        foreach (var line in lines)
        {
            if (line.Contains("Installing Star Citizen"))
            {
                var strstart = line.IndexOf(" at ") + " at ".Length;
                var strend = line.LastIndexOf("StarCitizen") + "StarCitizen".Length;
                var installDirectory = line.Substring(strstart, strend - strstart);
                return installDirectory;
            }
        }
        _logger.LogError("Failed to find SC install directory from launcher log");
        return String.Empty;
    }
}

public sealed class StarCitizenInstallationViewModel
{
    public required string ChannelName { get; init; }
    public required string Path { get; init; }
    public BuildManifest? Manifest { get; set; }
    
    public string DisplayVersion => $"{ChannelName} - {Manifest?.Data.Branch}-{Manifest?.Data.RequestedP4ChangeNum}";
}

public class BuildManifest
{
    public BuildManifestData Data { get; set; }
}

public class BuildManifestData
{
    public string? Branch { get; set; }
    public string? BuildDateStamp { get; set; }
    public string? BuildId { get; set; }
    public string? BuildTimeStamp { get; set; }
    public string? Config { get; set; }
    public string? Platform { get; set; }
    public string? RequestedP4ChangeNum { get; set; }
    public string? Shelved_Change { get; set; }
    public string? Tag { get; set; }
    public string? Version { get; set; }
}

[JsonSerializable(typeof(BuildManifest))]
internal partial class StarBreakerSerializerContext : JsonSerializerContext;