using System.Collections.ObjectModel;
using System.Text.Json;
using System.Text.Json.Serialization;
using Avalonia.Platform.Storage;
using Avalonia.Threading;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Microsoft.Extensions.Logging;
using StarBreaker.Services;

namespace StarBreaker.Screens;

public sealed partial class SplashWindowViewModel : ViewModelBase
{
    private readonly ILogger<SplashWindowViewModel> _logger;
    private readonly IP4kService _p4kService;

    [ObservableProperty] private ObservableCollection<StarCitizenInstallationViewModel> _installations;
    [ObservableProperty] private double? _progress;
    [ObservableProperty] private string _loadingText;

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
        LoadingText = "Loading P4k file...";
        var p4kLoadProgress = new Progress<double>(x => Dispatcher.UIThread.Post(() =>
        {
            LoadingText = "Loading P4k file...";
            Progress = x;
        }));

        var fileSystemProgress = new Progress<double>(x => Dispatcher.UIThread.Post(() =>
        {
            LoadingText = "Loading file system...";
            Progress = x;
        }));

        await Task.Run(() => _p4kService.OpenP4k(path, p4kLoadProgress, fileSystemProgress));

        OnP4kLoaded();
    }

    [RelayCommand]
    public async Task PickP4k()
    {
        _logger?.LogTrace("PickP4k enter");
        var defaultPath = await App.StorageProvider.TryGetFolderFromPathAsync(Constants.DefaultStarCitizenFolder);
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
        if (!TryGetInstallDirectory(out var currentInstallDirectory))
            currentInstallDirectory = Constants.DefaultStarCitizenFolder;

        GetP4ksFromDirectory(currentInstallDirectory);
    }

    private void GetP4ksFromDirectory(string installationPath)
    {
        if (!Directory.Exists(installationPath))
            return;

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
                    manifest = JsonSerializer.Deserialize<BuildManifest>(
                        File.ReadAllText(Path.Combine(directoryName, Constants.BuildManifest))
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
    private bool TryGetInstallDirectory(out string dir)
    {
        dir = "";

        var launcherPath = Constants.DefaultRSILauncherFolder;
        if (!Directory.Exists(launcherPath))
        {
            _logger.LogError("Failed to find RSI Launcher directory");
            return false;
        }

        var logPath = Path.Combine(launcherPath, "logs", "log.log");

        if (!File.Exists(logPath))
        {
            _logger.LogError("Failed to find RSI Launcher log");
            return false;
        }

        foreach (var line in File.ReadLines(logPath))
        {
            if (!line.Contains("Installing Star Citizen"))
                continue;

            try
            {
                var strstart = line.IndexOf(" at ", StringComparison.InvariantCultureIgnoreCase) + " at ".Length;
                var strend = line.LastIndexOf("StarCitizen", StringComparison.InvariantCultureIgnoreCase) + "StarCitizen".Length;
                var installDirectory = line.Substring(strstart, strend - strstart);
                dir = installDirectory;
                return true;
            }
            catch (Exception e)
            {
                _logger.LogError(e, "Failed to parse SC install directory from launcher log");
                return false;
            }
        }

        _logger.LogError("Failed to find SC install directory from launcher log");
        return false;
    }
}

public sealed class StarCitizenInstallationViewModel
{
    public required string ChannelName { get; init; }
    public required string Path { get; init; }
    public BuildManifest? Manifest { get; set; }

    public string DisplayVersion => $"{ChannelName} - {Manifest?.Data?.Branch}-{Manifest?.Data?.RequestedP4ChangeNum}";
}

public class BuildManifest
{
    public BuildManifestData? Data { get; set; }
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