﻿namespace StarBreaker.Sandbox;

//todo remove this
public static class SandboxPaths
{
    public static readonly string StarCitizenFolder = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), "Roberts Space Industries", "StarCitizen");
    public static readonly string StarCitizenCharactersFolder = Path.Combine(StarCitizenFolder, "PTU", "user", "client", "0", "CustomCharacters");

    public static readonly string ResearchFolder = Path.GetFullPath(Path.Combine(Environment.CurrentDirectory, "..", "..", "..", "..", "..", "research"));

    public static readonly string WebsiteCharacters = Path.Combine(ResearchFolder, "websiteCharacters");
    public static readonly string LocalCharacters = Path.Combine(ResearchFolder, "localCharacters");
    public static readonly string ModdedCharacters = Path.Combine(ResearchFolder, "moddedCharacters");
}