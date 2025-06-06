﻿using System.Net.Http.Json;
using CliFx;
using CliFx.Attributes;
using CliFx.Infrastructure;

// ReSharper disable ClassNeverInstantiated.Global
// ReSharper disable UnusedAutoPropertyAccessor.Global

namespace StarBreaker.Cli;

[Command("chf-download", Description = "Downloads all characters from the website and saves them to the website characters folder.")]
public class DownloadCommand : ICommand
{
    [CommandOption("OutputFolder", Description = "Download folder", EnvironmentVariable = "OUTPUT_FOLDER")]
    public required string OutputFolder { get; init; }

    public async ValueTask ExecuteAsync(IConsole console)
    {
        if (string.IsNullOrWhiteSpace(OutputFolder))
        {
            await console.Error.WriteLineAsync("Folder is required");
            return;
        }

        using var http = new HttpClient();
        http.DefaultRequestHeaders.Add("User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/58.0.3029.110 Safari/537.3");

        await console.Output.WriteLineAsync("Downloading all missing characters...");

        await foreach (var row in GetCharactersAsync(http))
        {
            try
            {
                if (!row.dnaUrl!.Contains("chf") || !row.previewUrl!.Contains("jpeg"))
                {
                    await console.Output.WriteLineAsync($"Skipping {row.title}, invalid urls");
                }

                var thisCharacterFolder = Path.Combine(OutputFolder, GetSafeDirectoryName(row));
                Directory.CreateDirectory(thisCharacterFolder);

                var imageFilename = row.previewUrl!.Split('/').Last();
                var dnaFilename = row.dnaUrl.Split('/').Last();
                var imageFile = Path.Combine(thisCharacterFolder, imageFilename);
                var dnaFile = Path.Combine(thisCharacterFolder, dnaFilename);
                if (File.Exists(imageFile) && File.Exists(dnaFile))
                {
                    await console.Output.WriteLineAsync($"Skipping {row.title}, already downloaded");
                    continue;
                }

                await console.Output.WriteLineAsync($"Downloading {row.title}...");

                await Task.WhenAll(DownloadFileAsync(http, row.dnaUrl, dnaFile), DownloadFileAsync(http, row.previewUrl, imageFile));

                await console.Output.WriteLineAsync($"Downloaded {row.title}");
            }
            catch (Exception e)
            {
                await console.Output.WriteLineAsync($"Error downloading {row.title}: {e.Message}");
            }
        }
    }
    
    private static async IAsyncEnumerable<SccCharacter> GetCharactersAsync(HttpClient httpClient)
    {
        var page = 1;
        while (true)
        {
            var response = await httpClient.GetFromJsonAsync<SccRoot>(
                $"https://www.star-citizen-characters.com/api/heads?page={page++}&orderBy=latest"
            );
            foreach (var row in response!.body!.rows!)
            {
                yield return row;
            }

            if (response.body.hasNextPage == false)
                break;
        }
    }

    private static async Task DownloadFileAsync(HttpClient httpClient, string url, string path)
    {
        var stream = await httpClient.GetStreamAsync(url);
        await using var fileStream = File.Create(path);
        await stream.CopyToAsync(fileStream);
    }

    public static string GetSafeDirectoryName(SccCharacter character)
    {
        var start = character.title;

        Array.ForEach([..Path.GetInvalidPathChars(), ' '], x => start = start!.Replace(x, '_'));

        return $"{start}-{character.id![..8]}";
    }

    public class SccRoot
    {
        public SccBody? body { get; set; }
    }

    public class SccBody
    {
        public bool hasNextPage { get; set; }
        public SccCharacter[]? rows { get; set; }
    }

    public class SccCharacter
    {
        public string? id { get; set; }
        public string? title { get; set; }
        public string? previewUrl { get; set; }
        public string? dnaUrl { get; set; }
    }
}